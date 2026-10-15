"""Shared parts of the Twitch test tools in this folder: where things are, a redactor that
keeps tokens out of everything the tools print or write, a small Helix client for the
few requests the tools make themselves, and the check that keeps the harness out of
PerritoDeck's real data folder. Standard library only; MIT, like the rest of plugins/.
"""

import hashlib
import importlib
import json
import os
import re
import stat
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
PLUGINS = os.path.dirname(HERE)
REPO = os.path.dirname(PLUGINS)
TWITCH_DIR = os.path.join(PLUGINS, "twitch")
PLUGIN_ID = "dev.perritodeck.twitch"
ID_URL = "https://id.twitch.tv"
HELIX_URL = "https://api.twitch.tv/helix"
CLIENT_ID_ENV = "PERRITODECK_TWITCH_CLIENT_ID"
TIMEOUT = 10
# The harness's test folder unless --data-dir says otherwise. The cleanup script looks
# for the sign-in there too.
DEFAULT_TEST_FOLDER = os.path.join(tempfile.gettempdir(), "perritodeck-twitch-live-test")
MARKER = ".perritodeck-twitch-live-test"  # in every test folder the harness made


def plugin_data(folder):
    """Where a test folder keeps the plugin's own files (as the host lays it out)."""
    return os.path.join(folder, "plugin-data", PLUGIN_ID)


def manifest():
    with open(os.path.join(TWITCH_DIR, "perritodeck-plugin.json"), encoding="utf-8") as f:
        return json.load(f)


_plugin = {}


def plugin_module():
    """plugins/twitch/plugin.py, imported (not run), for its constants: the scopes the
    sign-in asks for, the placeholder. Importing it starts nothing."""
    if "module" not in _plugin:
        if TWITCH_DIR not in sys.path:
            sys.path.insert(0, TWITCH_DIR)
        _plugin["module"] = importlib.import_module("plugin")
    return _plugin["module"]


# --- keeping secrets out ----------------------------------------------------------------

# Keys whose values are secrets wherever they turn up: JSON bodies, form fields, query
# strings. The client ID isn't one (Twitch calls it public), so it stays readable.
SECRET_KEYS = {"access_token", "refresh_token", "device_code", "client_secret", "token", "id_token",
               "user_code", "device-code", "password"}
SECRET_HEADERS = {"authorization", "cookie", "set-cookie"}
REDACTED = "[redacted]"
# "Bearer <token>" or "OAuth <token>" in text, for a token not seen before. Only something
# shaped like a token (8+ characters with a digit), so Twitch's own wording ("Invalid
# OAuth token") stays readable.
_SCHEME_TOKEN = re.compile(r"\b(Bearer|OAuth)\s+(?=[A-Za-z0-9._~+/=-]*\d)[A-Za-z0-9._~+/=-]{8,}", re.I)


class Redactor:
    """Replaces secrets with [redacted]: values under SECRET_KEYS, the token in an
    Authorization header, and any secret it has seen before wherever else it turns up
    (an error message, a URL)."""

    def __init__(self):
        self.secrets = set()

    def learn(self, value):
        if isinstance(value, str) and len(value) >= 6 and value != REDACTED:
            self.secrets.add(value)

    def learn_tokens(self, tokens):
        """Remember the secrets of a tokens.json."""
        for key in ("access_token", "refresh_token"):
            self.learn((tokens or {}).get(key))

    def learn_all(self, obj, key=None):
        """Walk obj and remember every secret in it, before anything is written."""
        if isinstance(obj, dict):
            for k, v in obj.items():
                self.learn_all(v, k)
        elif isinstance(obj, (list, tuple)):
            for v in obj:
                self.learn_all(v, key)
        elif isinstance(obj, str) and key is not None:
            lower = str(key).lower()
            if lower in SECRET_KEYS:
                self.learn(obj)
            elif lower in SECRET_HEADERS:
                m = re.match(r"^\s*(?:Bearer|OAuth)\s+(\S+)", obj, re.I)
                self.learn(m.group(1) if m else obj)

    def text(self, s):
        if not isinstance(s, str):
            return s
        for secret in sorted(self.secrets, key=len, reverse=True):
            if secret in s:
                s = s.replace(secret, REDACTED)
        return _SCHEME_TOKEN.sub(lambda m: m.group(1) + " " + REDACTED, s)

    def url(self, url):
        if not isinstance(url, str):
            return url
        try:
            parts = urllib.parse.urlsplit(url)
        except ValueError:
            return self.text(url)
        if parts.query:
            pairs = [(k, REDACTED if k.lower() in SECRET_KEYS else v)
                     for k, v in urllib.parse.parse_qsl(parts.query, keep_blank_values=True)]
            url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(pairs, safe="@:/,")))
        return self.text(url)

    def value(self, obj, key=None):
        """A copy of obj with every secret replaced."""
        if isinstance(obj, dict):
            return {k: self.value(v, k) for k, v in obj.items()}
        if isinstance(obj, (list, tuple)):
            return [self.value(v, key) for v in obj]
        if isinstance(obj, str):
            lower = str(key).lower() if key is not None else ""
            if lower in SECRET_KEYS:
                return REDACTED
            if lower in SECRET_HEADERS:
                m = re.match(r"^\s*(Bearer|OAuth)\s", obj, re.I)
                return f"{m.group(1)} {REDACTED}" if m else REDACTED
            if lower.endswith("url") or obj.startswith(("http://", "https://", "ws://", "wss://")):
                return self.url(obj)
            return self.text(obj)
        return obj

    def scrub(self, obj):
        self.learn_all(obj)
        return self.value(obj)


def token_key(token):
    """A short fingerprint of a token, for telling tokens apart without keeping them."""
    return hashlib.sha256((token or "").encode()).hexdigest()[:16]


# --- tokens.json --------------------------------------------------------------------------


def read_tokens(path):
    """The plugin's tokens.json as a dict, or None."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return None
    return data if isinstance(data, dict) and data.get("access_token") else None


def write_json_atomically(path, data, tries=40):
    """Write JSON through a temporary file. On Windows, replacing a file fails while any
    other handle has it open (the plugin reads tokens.json often), so it tries again."""
    tmp = path + ".tools-tmp"
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(data, f)
    for attempt in range(tries):
        try:
            os.replace(tmp, path)
            return
        except PermissionError:
            if attempt == tries - 1:
                raise
            time.sleep(0.05)


# --- a small Helix client -------------------------------------------------------------------


class HelixError(RuntimeError):
    """Twitch couldn't be reached."""


def http_json(method, url, token=None, client_id=None, body=None, form=None, scheme="Bearer", timeout=TIMEOUT):
    """One request; returns (status, JSON answer or {}, headers). 4xx and 5xx answers are
    returned, not raised; only an unreachable server raises HelixError."""
    data = None
    req = urllib.request.Request(url, method=method)
    if form is not None:
        data = urllib.parse.urlencode(form).encode()
        req.add_header("Content-Type", "application/x-www-form-urlencoded")
    elif body is not None:
        data = json.dumps(body).encode()
        req.add_header("Content-Type", "application/json")
    if token:
        req.add_header("Authorization", f"{scheme} {token}")
    if client_id:
        req.add_header("Client-Id", client_id)
    try:
        with urllib.request.urlopen(req, data=data, timeout=timeout) as resp:
            status, raw, headers = resp.status, resp.read(), resp.headers
    except urllib.error.HTTPError as e:
        try:
            status, raw, headers = e.code, e.read() or b"", e.headers
        finally:
            e.close()
    except (urllib.error.URLError, OSError) as e:
        raise HelixError(f"couldn't reach Twitch: {getattr(e, 'reason', e)}") from None
    try:
        answer = json.loads(raw.decode("utf-8") or "{}")
    except ValueError:
        answer = {"message": raw.decode("utf-8", "replace")[:300]}
    return status, answer if isinstance(answer, dict) else {"data": answer}, headers


def helix(method, path, params=None, token=None, client_id=None, base=HELIX_URL, body=None):
    url = base + path
    if params:
        url += "?" + urllib.parse.urlencode(params, doseq=True)
    return http_json(method, url, token=token, client_id=client_id, body=body)


def message_of(answer):
    return str((answer or {}).get("message") or (answer or {}).get("error") or "")


LOGIN = re.compile(r"^[a-z0-9_]{1,25}$")


def login_key(name):
    """A channel or user name as Twitch compares it: trimmed, lower case, no @."""
    return (name or "").strip().lower().lstrip("@")


# --- keeping out of the real data folder ---------------------------------------------------


def real_data_roots():
    """PerritoDeck's real per-user folders, which the harness must never use: the host's
    settings folder (and PERRITODECK_DATA_DIR when set) and the SDK's fallback folder for
    plugins run by hand."""
    home = os.path.expanduser("~")
    roots = []
    if os.environ.get("PERRITODECK_DATA_DIR"):
        roots.append(os.environ["PERRITODECK_DATA_DIR"])
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA") or os.path.join(home, "AppData", "Roaming")
        local = os.environ.get("LOCALAPPDATA") or os.path.join(home, "AppData", "Local")
        roots += [os.path.join(appdata, "perritodeck"), os.path.join(local, "PerritoDeck")]
    elif sys.platform == "darwin":
        support = os.path.join(home, "Library", "Application Support")
        roots += [os.path.join(support, "dev.perritodeck.PerritoDeck"), os.path.join(support, "PerritoDeck")]
    else:
        config = os.environ.get("XDG_CONFIG_HOME") or os.path.join(home, ".config")
        data = os.environ.get("XDG_DATA_HOME") or os.path.join(home, ".local", "share")
        roots += [os.path.join(config, "perritodeck"), os.path.join(data, "perritodeck")]
    return roots


def _norm(path):
    return os.path.normcase(os.path.realpath(os.path.abspath(path)))


def inside(path, root):
    path, root = _norm(path), _norm(root)
    try:
        return os.path.commonpath([path, root]) == root
    except ValueError:  # different drives
        return False


def ownership_problem(path, st, uid):
    """POSIX: why an existing test folder isn't safely the user's own, from its os.stat()
    result, or None. On a shared /tmp another user could have made the folder first."""
    if st.st_uid != uid:
        return f"{path} belongs to another user (uid {st.st_uid}); give the harness a folder of your own"
    mode = stat.S_IMODE(st.st_mode)
    if mode & 0o022:
        return (f"{path} can be written to by other users (mode {oct(mode)}); "
                "give the harness a folder only you can write to")
    return None


def folder_problem(path):
    """Why a folder can't be the harness's test folder, or None."""
    for root in real_data_roots():
        if inside(path, root) or inside(root, path):
            return (f"{path} is, or holds, PerritoDeck's real data folder ({root}); "
                    "give the harness a separate test folder")
    if inside(path, REPO):
        return f"{path} is inside the repository, where a token could get committed; use a folder outside it"
    if os.name == "posix" and os.path.islink(path):
        return f"{path} is a symbolic link; give the harness a real folder"
    if os.name == "posix" and os.path.isdir(path):
        problem = ownership_problem(path, os.stat(path), os.getuid())
        if problem:
            return problem
    if os.path.isdir(path) and os.listdir(path) and not os.path.exists(os.path.join(path, MARKER)):
        return f"{path} already has files the harness didn't put there; give it an empty or new folder"
    return None


def location_problem(path):
    """Why a file the tools write (a trace, a report) can't go at path, or None: never in
    PerritoDeck's real data folders or the repository. Both files name the accounts and
    the client ID, and a report can be committed by mistake."""
    folder = os.path.dirname(os.path.abspath(path))
    for root in real_data_roots():
        if inside(folder, root):
            return f"{path} is inside PerritoDeck's real data folder ({root}); write it somewhere else"
    if inside(folder, REPO):
        return f"{path} is inside the repository, where it could get committed; write it outside"
    return None


def prepare_test_folder(path):
    """Create the test folder (and mark it as the harness's); raises ValueError if it
    can't be used. On macOS and Linux only the user can open it (mode 700)."""
    problem = folder_problem(path)
    if problem:
        raise ValueError(problem)
    if os.name == "posix":
        os.makedirs(path, mode=0o700, exist_ok=True)
        problem = folder_problem(path)  # someone else's, made in the meantime?
        if problem:
            raise ValueError(problem)
        os.chmod(path, 0o700)
    os.makedirs(plugin_data(path), mode=0o700, exist_ok=True)
    with open(os.path.join(path, MARKER), "w", encoding="utf-8") as f:
        f.write("A test folder of plugins/tools/twitch_live_test.py. It may hold a Twitch sign-in "
                "(plugin-data/dev.perritodeck.twitch/tokens.json): delete the folder when you're done.\n")
    return path
