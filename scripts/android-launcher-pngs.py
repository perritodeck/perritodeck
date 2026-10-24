#!/usr/bin/env python3
"""Make Perrito's launcher icon bitmaps for Android 5 to 7 (API 22 to 25).

    py -3 scripts/android-launcher-pngs.py     (needs Pillow)

Scales docs/assets/icon-rounded-1024.png (icon-master-rounded.svg) to 48 dp at each density,
writes the set to docs/assets/android/drawable-<density>/ic_launcher.png and copies it into the
app's res/. Android 8 and newer use the adaptive icon in drawable-anydpi-v26/ instead.

Why bitmaps: on API 22 and 23 a vector is drawn at the screen's own density whatever density
the launcher asks for, and launchers don't scale icons up, so on a 160 dpi tablet whose
launcher asks for 240 dpi icons a 48 dp vector came out a third smaller than the other apps'.
LauncherIconTest checks the result.
"""
import shutil
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SOURCE = REPO / "docs/assets/icon-rounded-1024.png"
ASSETS = REPO / "docs/assets/android"
RES = REPO / "android/app/src/main/res"
SIZES = {"mdpi": 48, "hdpi": 72, "xhdpi": 96, "xxhdpi": 144, "xxxhdpi": 192}


def main() -> None:
    master = Image.open(SOURCE).convert("RGBA")
    for density, size in SIZES.items():
        rel = Path(f"drawable-{density}/ic_launcher.png")
        out = ASSETS / rel
        out.parent.mkdir(parents=True, exist_ok=True)
        master.resize((size, size), Image.Resampling.LANCZOS).save(out, optimize=True)
        (RES / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(out, RES / rel)
        print(f"{rel}: {size}x{size}, {out.stat().st_size} bytes")


if __name__ == "__main__":
    main()
