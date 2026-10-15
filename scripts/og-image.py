#!/usr/bin/env python3
"""Make website/img/og.png, the 1200x630 picture link previews show (Open Graph and
Twitter cards).

    python3 scripts/og-image.py                       # rewrite website/img/og.png
    python3 scripts/og-image.py <repo root> <out.png> # read from another checkout, write elsewhere

Inputs: docs/assets/site/devices-hero.png (the device mockup master),
docs/assets/icon-rounded-1024.png (the app icon) and
host/crates/render/assets/Inter-SemiBold.otf (SIL Open Font License; Pillow can't read the
site's woff2 fonts). Colours are the light theme's in website/style.css. Needs Pillow. The
same inputs give the same bytes with the same Pillow and zlib (og.png was last made with
Pillow 12.3 on Windows), so compare the picture rather than the bytes after an upgrade.
"""

import os
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = sys.argv[1] if len(sys.argv) > 1 else os.path.normpath(os.path.join(HERE, ".."))
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(ROOT, "website", "img", "og.png")

W, H = 1200, 630
FOG = (0xEE, 0xF1, 0xF6)
INK = (0x1C, 0x24, 0x33)
INK_SOFT = (0x4A, 0x55, 0x68)
APRICOT = (0xFA, 0xB3, 0x87)
APRICOT_DEEP = (0xC9, 0x62, 0x2A)

FONT = os.path.join(ROOT, "host", "crates", "render", "assets", "Inter-SemiBold.otf")
HERO = os.path.join(ROOT, "docs", "assets", "site", "devices-hero.png")
ICON = os.path.join(ROOT, "docs", "assets", "icon-rounded-1024.png")

LEFT = 64
HERO_W = 700


def font(size):
    return ImageFont.truetype(FONT, size)


def wrap(draw, text, f, width):
    lines, line = [], ""
    for word in text.split():
        trial = f"{line} {word}".strip()
        if draw.textlength(trial, font=f) <= width or not line:
            line = trial
        else:
            lines.append(line)
            line = word
    lines.append(line)
    return lines


def main():
    img = Image.new("RGB", (W, H), FOG)

    # A soft apricot glow behind the devices, the site's one warm colour.
    glow = Image.new("L", (W, H), 0)
    ImageDraw.Draw(glow).ellipse((W - HERO_W - 10, 40, W + 120, H + 60), fill=110)
    glow = glow.filter(ImageFilter.GaussianBlur(90))
    img.paste(Image.new("RGB", (W, H), APRICOT), (0, 0), glow)

    hero = Image.open(HERO).convert("RGBA")
    hero_h = round(hero.height * HERO_W / hero.width)
    hero = hero.resize((HERO_W, hero_h), Image.LANCZOS)
    hx, hy = W - HERO_W - 28, (H - hero_h) // 2 + 6
    img.paste(hero, (hx, hy), hero)

    draw = ImageDraw.Draw(img)
    column = hx - LEFT - 24

    # The icon and the name.
    icon = Image.open(ICON).convert("RGBA").resize((72, 72), Image.LANCZOS)
    img.paste(icon, (LEFT, 64), icon)
    name = font(40)
    draw.text((LEFT + 72 + 18, 64 + 36), "PerritoDeck", font=name, fill=INK, anchor="lm")

    # The tagline, as on the home page.
    big = font(54)
    y = 200
    for line in wrap(draw, "Your old tablet is a control deck now.", big, column):
        draw.text((LEFT, y), line, font=big, fill=INK)
        y += 62

    small = font(25)
    y += 22
    for line in ("Free and open source.", "No account, no cloud."):
        draw.text((LEFT, y), line, font=small, fill=INK_SOFT)
        y += 36

    draw.text((LEFT, H - 64), "perritodeck.dev", font=font(26), fill=APRICOT_DEEP, anchor="ls")

    os.makedirs(os.path.dirname(os.path.abspath(OUT)), exist_ok=True)
    img.save(OUT, optimize=True)
    print(OUT, os.path.getsize(OUT), "bytes")


if __name__ == "__main__":
    main()
