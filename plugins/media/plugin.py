"""Now playing and media controls through playerctl (MPRIS, Linux)."""

import os
import shutil
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from perritodeck_plugin import Plugin  # noqa: E402

plugin = Plugin()
NOTHING = "–"
PLAYERCTL = shutil.which("playerctl")


def playerctl(*args):
    if not PLAYERCTL:
        raise RuntimeError("playerctl isn't installed (e.g. sudo apt install playerctl)")
    out = subprocess.run([PLAYERCTL, *args], capture_output=True, text=True, timeout=3)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "no player is running")
    return out.stdout.strip()


def describe(artist, title):
    if artist and title:
        return f"{artist} – {title}"
    return title or artist or NOTHING


def poll():
    last = None
    while True:
        try:
            status = playerctl("status").lower()
            title = describe(playerctl("metadata", "artist"), playerctl("metadata", "title"))
        except Exception:
            status, title = "stopped", NOTHING
        if (status, title) != last:
            plugin.set_value("status", status)
            plugin.set_value("title", title)
            last = (status, title)
        time.sleep(2)


@plugin.on_start
def start():
    if not PLAYERCTL:
        plugin.log("playerctl isn't installed, so nothing can be shown or controlled", "warn")
    threading.Thread(target=poll, daemon=True).start()


@plugin.action("play-pause")
def play_pause(settings):
    playerctl("play-pause")


@plugin.action("next")
def next_track(settings):
    playerctl("next")


@plugin.action("previous")
def previous_track(settings):
    playerctl("previous")


if __name__ == "__main__":
    plugin.run()
