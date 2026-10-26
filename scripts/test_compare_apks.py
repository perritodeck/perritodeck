"""Tests for compare_apks.py, on small APK-shaped zips it writes itself, with an ART profile and a
dex file made here byte by byte.

    cd scripts && python3 -m unittest test_compare_apks
"""

import io
import os
import struct
import tempfile
import unittest
import unittest.mock
import zipfile
import zlib

import compare_apks as ca


def make_dex(types, methods):
    """A dex file with just the parts compare_apks.Dex reads: strings, types, protos, methods.
    types: descriptors; methods: (class type index, name, return type index, [param type indices])."""
    strings = sorted(set(types) | {m[1] for m in methods} | {"V"})
    string_idx = {s: i for i, s in enumerate(strings)}
    protos = sorted({(m[2], tuple(m[3])) for m in methods})
    proto_idx = {p: i for i, p in enumerate(protos)}
    header = 0x70
    string_ids_off = header
    type_ids_off = string_ids_off + 4 * len(strings)
    proto_ids_off = type_ids_off + 4 * len(types)
    method_ids_off = proto_ids_off + 12 * len(protos)
    data_off = method_ids_off + 8 * len(methods)
    data = bytearray()
    string_offsets = []
    for s in strings:
        string_offsets.append(data_off + len(data))
        data += bytes([len(s)]) + s.encode() + b"\0"
    while len(data) % 4:
        data += b"\0"
    param_offsets = {}
    for p in protos:
        if p[1]:
            param_offsets[p] = data_off + len(data)
            data += struct.pack("<I", len(p[1])) + struct.pack(f"<{len(p[1])}H", *p[1])
            while len(data) % 4:
                data += b"\0"
    out = bytearray(header)
    out[:8] = b"dex\n035\0"
    struct.pack_into("<I", out, 8, 0x12345678)
    struct.pack_into("<IIIIII", out, 0x38, len(strings), string_ids_off, len(types), type_ids_off,
                     len(protos), proto_ids_off)
    struct.pack_into("<II", out, 0x58, len(methods), method_ids_off)
    for off in string_offsets:
        out += struct.pack("<I", off)
    for t in types:
        out += struct.pack("<I", string_idx[t])
    for p in protos:
        out += struct.pack("<III", string_idx["V"], p[0], param_offsets.get(p, 0))
    for cls, name, ret, params in methods:
        out += struct.pack("<HHI", cls, proto_idx[(ret, tuple(params))], string_idx[name])
    return bytes(out + data)


def make_profile(dexes):
    """An ART profile, version 010. dexes: (key, checksum, method count, classes, hot, startup, post)."""
    headers, bodies = b"", b""
    for key, checksum, methods, classes, hot, startup, post in dexes:
        hot_region = b""
        last = 0
        for m in sorted(hot):
            hot_region += struct.pack("<HH", m - last, 0)
            last = m
        class_region = b""
        last = 0
        for c in sorted(classes):
            class_region += struct.pack("<H", c - last)
            last = c
        bitmap = bytearray((2 * methods + 7) // 8)
        for offset, flagged in ((0, startup), (methods, post)):
            for m in flagged:
                bitmap[(offset + m) // 8] |= 1 << ((offset + m) % 8)
        headers += struct.pack("<HHIII", len(key), len(classes), len(hot_region), checksum, methods) + key.encode()
        bodies += hot_region + class_region + bytes(bitmap)
    body = headers + bodies
    compressed = zlib.compress(body)
    return b"pro\x00" + b"010\x00" + bytes([len(dexes)]) + struct.pack("<II", len(body), len(compressed)) + compressed


TYPES = ["La/A;", "La/B;", "La/C;", "I"]
METHODS = [(0, "<init>", 3, []), (1, "<init>", 0, [3]), (2, "run", 3, [0, 3])]
DEX = make_dex(TYPES, METHODS)


def profile(classes, hot):
    return make_profile([("classes.dex", 0x12345678, len(METHODS), classes, hot, hot, hot)])


BASE = {
    "AndroidManifest.xml": b"<manifest/>",
    "classes.dex": DEX,
    "assets/dexopt/baseline.prof": profile({0, 1}, {0, 1}),
    "assets/dexopt/baseline.profm": b"profm",
    "res/a.png": b"\x89PNG",
}


class CompareApksTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def apk(self, name, files=None, order=None, stored=(), extra=None):
        files = dict(BASE if files is None else files)
        if extra:
            files.update(extra)
        path = os.path.join(self.tmp.name, name)
        with zipfile.ZipFile(path, "w") as z:
            for entry in order or files:
                info = zipfile.ZipInfo(entry, date_time=(1981, 1, 1, 1, 1, 2))
                info.compress_type = zipfile.ZIP_STORED if entry in stored else zipfile.ZIP_DEFLATED
                z.writestr(info, files[entry])
        return path

    def run_compare(self, a, b):
        lines = []
        same = ca.compare(a, b, out=lines.append)
        return same, "\n".join(lines)

    def test_identical_unsigned_builds_match(self):
        same, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk"))
        self.assertTrue(same, text)
        self.assertIn("the files are identical", text)
        self.assertIn("5 of 5 entries match", text)
        self.assertTrue(text.endswith("MATCH"))

    def test_v1_signature_files_are_left_out(self):
        signed = self.apk("signed.apk", extra={"META-INF/MANIFEST.MF": b"m", "META-INF/CERT.SF": b"s",
                                               "META-INF/CERT.RSA": b"r"})
        same, text = self.run_compare(signed, self.apk("b.apk"))
        self.assertTrue(same, text)
        self.assertIn("(signature left out)", text)
        self.assertNotIn("identical", text)  # a signed file can't be byte for byte the same

    def test_other_meta_inf_files_count(self):
        same, text = self.run_compare(self.apk("a.apk", extra={"META-INF/x.version": b"1"}), self.apk("b.apk"))
        self.assertFalse(same)
        self.assertIn("only in A: META-INF/x.version", text)

    def test_a_different_entry_is_named(self):
        files = dict(BASE, **{"res/a.png": b"\x89PNG2"})
        same, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk", files))
        self.assertFalse(same)
        self.assertIn("DIFFERS res/a.png: contents (4 and 5 bytes)", text)
        self.assertTrue(text.endswith("DIFFERENT"))

    def test_compression_and_order_count(self):
        same, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk", stored={"res/a.png"}))
        self.assertFalse(same)
        self.assertIn("DIFFERS res/a.png: compression (8 and 0)", text)
        same, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk", order=sorted(BASE)))
        self.assertFalse(same)
        self.assertIn("the entries are in a different order", text)

    def test_the_zip_layout_counts_for_unsigned_builds(self):
        a = self.apk("a.apk")
        b = os.path.join(self.tmp.name, "b.apk")
        with zipfile.ZipFile(a) as za, zipfile.ZipFile(b, "w") as zb:
            for info in za.infolist():
                info.date_time = (2026, 10, 25, 0, 0, 0)
                zb.writestr(info, za.read(info))
        same, text = self.run_compare(a, b)
        self.assertFalse(same)
        self.assertIn("every entry matches, but the files differ", text)

    def test_a_profile_difference_names_the_classes_and_methods(self):
        files = dict(BASE, **{"assets/dexopt/baseline.prof": profile({0, 1, 2}, {0, 1, 2})})
        same, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk", files))
        self.assertFalse(same)
        self.assertIn("classes.dex: A has 2 classes, 2 hot methods, 2 startup and 2 post-startup; "
                      "B has 3, 3, 3 and 3", text)
        self.assertIn("classes only in B: 1\n      La/C;", text)
        self.assertIn("hot methods only in B: 1\n      La/C;->run(La/A;I)I", text)
        self.assertIn("post-startup methods only in B: 1", text)
        self.assertNotIn("only in A", text)

    def test_a_profile_for_another_dex_says_so(self):
        other = make_profile([("classes.dex", 0x9ABCDEF0, len(METHODS), {0}, {0}, {0}, {0})])
        files = dict(BASE, **{"assets/dexopt/baseline.prof": other})
        _, text = self.run_compare(self.apk("a.apk"), self.apk("b.apk", files))
        self.assertIn("checksums 12345678 and 9abcdef0): compare classes.dex first", text)

    def test_the_profile_decodes(self):
        p = ca.read_profile(make_profile([("classes.dex", 7, 10, {1, 4, 9}, {0, 5}, {0, 9}, {3})]))
        self.assertEqual(p, {"classes.dex": {"checksum": 7, "methods": 10, "classes": {1, 4, 9},
                                             "hot": {0, 5}, "startup": {0, 9}, "post": {3}}})
        with self.assertRaises(ca.ApkError):
            ca.read_profile(b"pro\x00" + b"015\x00" + bytes(9))

    def test_the_dex_names(self):
        dex = ca.Dex(DEX)
        self.assertEqual([dex.type(i) for i in range(4)], TYPES)
        self.assertEqual(dex.method(0), "La/A;-><init>()I")
        self.assertEqual(dex.method(1), "La/B;-><init>(I)La/A;")
        self.assertEqual(dex.method(2), "La/C;->run(La/A;I)I")

    def test_main_exit_codes(self):
        a, b = self.apk("a.apk"), self.apk("b.apk", dict(BASE, **{"res/a.png": b"x"}))
        with unittest.mock.patch("sys.stdout", io.StringIO()), unittest.mock.patch("sys.stderr", io.StringIO()):
            self.assertEqual(ca.main(["compare_apks.py", a, a]), 0)
            self.assertEqual(ca.main(["compare_apks.py", a, b]), 1)
            self.assertEqual(ca.main(["compare_apks.py", a, os.path.join(self.tmp.name, "none.apk")]), 2)
            self.assertEqual(ca.main(["compare_apks.py", a]), 2)


if __name__ == "__main__":
    unittest.main()
