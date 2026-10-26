#!/usr/bin/env python3
"""Compare two Perrito APKs entry by entry, leaving out the signature, to check that a release
build is reproducible (D11; docs/fdroid/README.md, Reproducible builds).

    python3 scripts/compare_apks.py a.apk b.apk

Either APK may be signed: the v1 signature files in META-INF (MANIFEST.MF, *.SF, *.RSA, *.DSA,
*.EC) are left out, and the APK Signing Block (v2 and v3) isn't a zip entry. For every other
entry it compares the name, the order, the compression method, the uncompressed contents and the
stored (compressed) bytes, and says which entries differ. When both APKs are unsigned it also
says whether the two files are byte for byte the same, which is what two builds of one commit
should give.

When assets/dexopt/baseline.prof differs, it decodes both profiles (the ART profile format
version 010, which AGP writes there) and lists the classes and methods only one of them names,
by their names in the APK's classes*.dex. The profile records each dex file's checksum, so it
also differs whenever a dex file does: then the dex is what to look at.

It exits 0 when every entry matches, 1 when something differs, and 2 when a file can't be read.
Standard library only.

This checks less than F-Droid does: F-Droid copies the signature of our APK onto its own
unsigned build and verifies the result (`apksigcopier compare`, or fdroidserver's verify_apks),
which needs the zip layout to match as well. For two unsigned builds, the byte comparison here
is the same test.
"""

import hashlib
import re
import struct
import sys
import zipfile
import zlib

SIGNATURE = re.compile(r"^META-INF/(MANIFEST\.MF|[^/]+\.(SF|RSA|DSA|EC))$")
PROFILE = "assets/dexopt/baseline.prof"
SHOWN = 20  # names listed for each side of a profile difference


class ApkError(Exception):
    pass


def entries(apk):
    """name -> (compression method, uncompressed size, SHA-256 of the contents, SHA-256 of the
    stored bytes), in zip order, signature files left out."""
    out = {}
    with open(apk, "rb") as raw, zipfile.ZipFile(apk) as z:
        for info in z.infolist():
            if SIGNATURE.match(info.filename):
                continue
            contents = hashlib.sha256(z.read(info)).hexdigest()
            raw.seek(info.header_offset)
            header = raw.read(30)
            if header[:4] != b"PK\x03\x04":
                raise ApkError(f"{apk}: no local header for {info.filename}")
            name_len, extra_len = struct.unpack("<HH", header[26:30])
            raw.seek(info.header_offset + 30 + name_len + extra_len)
            stored = hashlib.sha256(raw.read(info.compress_size)).hexdigest()
            out[info.filename] = (info.compress_type, info.file_size, contents, stored)
    return out


def is_signed(apk):
    with zipfile.ZipFile(apk) as z:
        if any(SIGNATURE.match(n) for n in z.namelist()):
            return True
    # The APK Signing Block sits just before the central directory and ends with this magic.
    with open(apk, "rb") as f:
        data = f.read()
    return b"APK Sig Block 42" in data[-(1 << 20) - 22:] if len(data) > 22 else False


# --- ART profiles -------------------------------------------------------------------------


def read_profile(data):
    """Decode an ART profile, format version 010 (what AGP writes to assets/dexopt/baseline.prof):
    {dex key: {"checksum", "methods" (method count of the dex), "classes", "hot", "startup",
    "post"}}, the last four being sets of type or method indices."""
    if data[:4] != b"pro\0":
        raise ApkError("not an ART profile")
    if data[4:8] != b"010\0":
        raise ApkError(f"ART profile version {data[4:7].decode('ascii', 'replace')}, not 010")
    count = data[8]
    size, compressed_size = struct.unpack_from("<II", data, 9)
    body = zlib.decompress(data[17:17 + compressed_size])
    if len(body) != size:
        raise ApkError("the profile's size doesn't match its header")
    pos = 0
    headers = []
    for _ in range(count):
        key_len, classes, hot_size, checksum, methods = struct.unpack_from("<HHIII", body, pos)
        pos += 16
        key = body[pos:pos + key_len].decode("utf-8")
        pos += key_len
        headers.append((key, classes, hot_size, checksum, methods))
    out = {}
    for key, classes, hot_size, checksum, methods in headers:
        hot, index, end = set(), 0, pos + hot_size
        while pos < end:
            diff, inline_caches = struct.unpack_from("<HH", body, pos)
            pos += 4
            if inline_caches:
                raise ApkError("inline caches in a baseline profile aren't supported here")
            index += diff
            hot.add(index)
        class_set, index = set(), 0
        for _ in range(classes):
            (diff,) = struct.unpack_from("<H", body, pos)
            pos += 2
            index += diff
            class_set.add(index)
        bitmap_len = (2 * methods + 7) // 8
        bitmap = body[pos:pos + bitmap_len]
        pos += bitmap_len

        def flagged(offset):
            return {m for m in range(methods) if bitmap[(offset + m) // 8] >> ((offset + m) % 8) & 1}

        out[key] = {"checksum": checksum, "methods": methods, "classes": class_set, "hot": hot,
                    "startup": flagged(0), "post": flagged(methods)}
    if pos != len(body):
        raise ApkError("the profile has bytes after its last dex file")
    return out


class Dex:
    """Just enough of a dex file to name its types and methods."""

    def __init__(self, data):
        if data[:4] != b"dex\n":
            raise ApkError("not a dex file")
        self.data = data
        (self.checksum,) = struct.unpack_from("<I", data, 8)
        u = lambda off: struct.unpack_from("<I", data, off)[0]  # noqa: E731
        self.string_ids, self.type_ids = (u(0x38), u(0x3C)), (u(0x40), u(0x44))
        self.proto_ids, self.method_ids = (u(0x48), u(0x4C)), (u(0x58), u(0x5C))

    def string(self, idx):
        (off,) = struct.unpack_from("<I", self.data, self.string_ids[1] + 4 * idx)
        while self.data[off] & 0x80:  # skip the uleb128 length
            off += 1
        end = self.data.index(b"\0", off + 1)
        return self.data[off + 1:end].decode("utf-8", "replace")

    def type(self, idx):
        if idx >= self.type_ids[0]:
            return f"<type {idx}>"
        (descriptor,) = struct.unpack_from("<I", self.data, self.type_ids[1] + 4 * idx)
        return self.string(descriptor)

    def method(self, idx):
        if idx >= self.method_ids[0]:
            return f"<method {idx}>"
        cls, proto, name = struct.unpack_from("<HHI", self.data, self.method_ids[1] + 8 * idx)
        _, ret, params_off = struct.unpack_from("<III", self.data, self.proto_ids[1] + 12 * proto)
        params = ""
        if params_off:
            (n,) = struct.unpack_from("<I", self.data, params_off)
            params = "".join(self.type(t) for t in struct.unpack_from(f"<{n}H", self.data, params_off + 4))
        return f"{self.type(cls)}->{self.string(name)}({params}){self.type(ret)}"


def dex_files(apk):
    with zipfile.ZipFile(apk) as z:
        return {n: z.read(n) for n in z.namelist() if re.fullmatch(r"classes\d*\.dex", n)}


def explain_profile(a_apk, b_apk):
    """Lines describing how the two APKs' baseline.prof differ."""
    with zipfile.ZipFile(a_apk) as za, zipfile.ZipFile(b_apk) as zb:
        a, b = read_profile(za.read(PROFILE)), read_profile(zb.read(PROFILE))
    dexes = {"A": dex_files(a_apk), "B": dex_files(b_apk)}
    lines = []
    for key in sorted(set(a) | set(b)):
        if key not in a or key not in b:
            lines.append(f"  {key}: only in {'A' if key in a else 'B'}'s profile")
            continue
        pa, pb = a[key], b[key]
        dex_name = key.rsplit("!", 1)[-1]
        lines.append(f"  {key}: A has {len(pa['classes'])} classes, {len(pa['hot'])} hot methods, "
                     f"{len(pa['startup'])} startup and {len(pa['post'])} post-startup; "
                     f"B has {len(pb['classes'])}, {len(pb['hot'])}, {len(pb['startup'])} and {len(pb['post'])}")
        if pa["checksum"] != pb["checksum"] or pa["methods"] != pb["methods"]:
            lines.append(f"    the profiles are for different dex files (checksums {pa['checksum']:08x} and "
                         f"{pb['checksum']:08x}): compare {dex_name} first")
            continue
        dex = None
        data = dexes["A"].get(dex_name)
        if data is not None:
            try:
                dex = Dex(data)
            except (ApkError, struct.error, IndexError):
                dex = None
        for what, namer in (("classes", "type"), ("hot", "method"), ("startup", "method"), ("post", "method")):
            for side, mine, theirs in (("A", pa, pb), ("B", pb, pa)):
                only = sorted(mine[what] - theirs[what])
                if not only:
                    continue
                label = {"classes": "classes", "hot": "hot methods", "startup": "startup methods",
                         "post": "post-startup methods"}[what]
                lines.append(f"    {label} only in {side}: {len(only)}")
                for idx in only[:SHOWN]:
                    lines.append(f"      {getattr(dex, namer)(idx) if dex else idx}")
                if len(only) > SHOWN:
                    lines.append(f"      ... and {len(only) - SHOWN} more")
    return lines


# --- main ---------------------------------------------------------------------------------


def compare(a_apk, b_apk, out=print):
    """Print the comparison and return True when every entry matches."""
    a, b = entries(a_apk), entries(b_apk)
    out(f"A {a_apk}: {len(a)} entries" + (" (signature left out)" if is_signed(a_apk) else ", unsigned"))
    out(f"B {b_apk}: {len(b)} entries" + (" (signature left out)" if is_signed(b_apk) else ", unsigned"))
    same = True
    for side, mine, theirs in (("A", a, b), ("B", b, a)):
        only = [n for n in mine if n not in theirs]
        if only:
            same = False
            out(f"only in {side}: {', '.join(only)}")
    differing = []
    for name in (n for n in a if n in b):
        (ca, sa, ha, ra), (cb, sb, hb, rb) = a[name], b[name]
        what = []
        if ha != hb:
            what.append(f"contents ({sa:,} and {sb:,} bytes)")
        if ca != cb:
            what.append(f"compression ({ca} and {cb})")
        if ha == hb and ca == cb and ra != rb:
            what.append("compressed bytes")
        if what:
            differing.append(name)
            out(f"DIFFERS {name}: {', '.join(what)}")
    if differing:
        same = False
    common_a = [n for n in a if n in b]
    common_b = [n for n in b if n in a]
    if common_a != common_b:
        same = False
        out("the entries are in a different order")
    out(f"{len(common_a) - len(differing)} of {len(set(a) | set(b))} entries match")
    if PROFILE in differing:
        out(f"{PROFILE}:")
        try:
            for line in explain_profile(a_apk, b_apk):
                out(line)
        except (ApkError, struct.error, zlib.error, IndexError, UnicodeDecodeError) as e:
            out(f"  couldn't decode it: {e}")
    if same and not is_signed(a_apk) and not is_signed(b_apk):
        with open(a_apk, "rb") as fa, open(b_apk, "rb") as fb:
            ha, hb = hashlib.sha256(fa.read()).hexdigest(), hashlib.sha256(fb.read()).hexdigest()
        if ha == hb:
            out(f"the files are identical: SHA-256 {ha}")
        else:
            same = False
            out(f"every entry matches, but the files differ (SHA-256 {ha} and {hb}): the zip layout does")
    out("MATCH" if same else "DIFFERENT")
    return same


def main(argv):
    if len(argv) != 3:
        print(__doc__.strip().split("\n\n")[1], file=sys.stderr)
        return 2
    try:
        return 0 if compare(argv[1], argv[2]) else 1
    except (OSError, zipfile.BadZipFile, ApkError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
