"""Windows: play sounds inside the plugin's own process, through winmm's MCI strings.

The Media Control Interface's "mpegvideo" device (DirectShow) plays WAV, MP3, WMA and
AAC with a volume, and starts in milliseconds, where a PowerShell player takes about a
second. Each sound gets its own alias and its own thread, and every MCI command for
that alias is sent from that thread: open, setaudio, play, status (polled until the end),
stop and close.

MciSound looks like the subprocess.Popen the soundboard already follows (communicate,
poll, terminate, returncode), so the rest of the plugin doesn't care which one plays.
Importable on every platform; winmm.dll is loaded on first use.
"""

import ctypes
import itertools
import threading
import time

POLL_SECONDS = 0.05
NEVER_STARTED_SECONDS = 5  # a device that never reports "playing" is given up on

_api = None
_api_lock = threading.Lock()
_aliases = itertools.count(1)


class MciError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


def _load():
    """(mciSendStringW, mciGetErrorStringW) with their prototypes. OSError off Windows."""
    global _api
    with _api_lock:
        if _api is None:
            try:
                from ctypes import wintypes

                winmm = ctypes.WinDLL("winmm")
            except (AttributeError, ValueError) as e:  # not Windows: no WinDLL
                raise OSError(f"winmm isn't available here ({e})") from None
            # MCIERROR mciSendStringW(LPCWSTR command, LPWSTR returnString, UINT cchReturn, HWND callback)
            send = winmm.mciSendStringW
            send.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, wintypes.UINT, wintypes.HWND]
            send.restype = wintypes.DWORD
            # BOOL mciGetErrorStringW(MCIERROR error, LPWSTR text, UINT cchText)
            error = winmm.mciGetErrorStringW
            error.argtypes = [wintypes.DWORD, wintypes.LPWSTR, wintypes.UINT]
            error.restype = wintypes.BOOL
            _api = (send, error)
    return _api


def mci(command):
    """Send one MCI command string and return its answer; raise MciError on failure."""
    send, error = _load()
    answer = ctypes.create_unicode_buffer(256)
    code = send(command, answer, len(answer), None)
    if code:
        text = ctypes.create_unicode_buffer(256)
        message = text.value if error(code, text, len(text)) else ""
        raise MciError(code, message or f"MCI error {code}")
    return answer.value


class MciSound:
    """One sound playing through MCI, shaped like a Popen.

    `volume` is 0–100. `send` is the MCI function (tests pass a fake). Raises OSError
    when winmm can't be loaded; a file MCI can't play ends with returncode 1 and the
    reason as stderr, like a player program that failed, so the next player is tried.
    """

    def __init__(self, path, volume, send=None):
        if send is None:
            _load()  # OSError here means "this player can't start"
            send = mci
        self._send = send
        self.alias = f"perritodeck{next(_aliases)}"
        self._stop = threading.Event()
        self._error = ""
        self.returncode = None
        self._thread = threading.Thread(target=self._run, args=(path, int(volume)), daemon=True)
        self._thread.start()

    def _run(self, path, volume):
        alias = self.alias
        opened = False
        try:
            if '"' in path:
                raise MciError(0, "the file name has a quote in it")
            self._send(f'open "{path}" type mpegvideo alias {alias}')
            opened = True
            try:
                # mpegvideo's volume runs from 0 to 1000.
                self._send(f"setaudio {alias} volume to {max(0, min(100, volume)) * 10}")
            except MciError:
                pass  # better at full volume than not at all
            self._send(f"play {alias}")
            begun = time.monotonic()
            seen_playing = False
            while not self._stop.wait(POLL_SECONDS):
                mode = self._send(f"status {alias} mode").strip().lower()
                if mode == "playing":
                    seen_playing = True
                elif mode == "stopped" and (seen_playing or time.monotonic() - begun > 1):
                    break  # it played to the end
                elif not seen_playing and time.monotonic() - begun > NEVER_STARTED_SECONDS:
                    raise MciError(0, f"it never started playing ({mode or 'no answer'})")
            self.returncode = 0
        except MciError as e:
            self._error = str(e)
            self.returncode = 1
        except Exception as e:  # never leave a sound "playing" forever
            self._error = f"{type(e).__name__}: {e}"
            self.returncode = 1
        finally:
            if opened:
                for command in (f"stop {alias}", f"close {alias}"):
                    try:
                        self._send(command)
                    except Exception:
                        pass

    def communicate(self, timeout=None):
        self._thread.join(timeout)
        return None, self._error.encode("utf-8")

    def wait(self, timeout=None):
        self._thread.join(timeout)
        return self.returncode

    def poll(self):
        return None if self._thread.is_alive() else self.returncode

    def terminate(self):
        self._stop.set()

    kill = terminate
