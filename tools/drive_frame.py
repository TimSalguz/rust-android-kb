#!/usr/bin/env python3
"""Draw a frame of `examples/drive.rs` (SESSION.frame) as the phone shows
it: the field above (the cursor │, the composing word [in brackets] and
underlined), the keyboard's draw list below — the same rectangles and
texts the Java side paints.

Usage: tools/drive_frame.py SESSION.frame OUT.png   (needs Pillow)
"""
import sys

from PIL import Image, ImageDraw, ImageFont

FONT = "/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf"
FALLBACKS = [
    FONT,
    "/nix/store/71mxn2pyq807r32qsd2mdszkajhlb39q-dejavu-fonts-2.37/share/fonts/truetype/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
]
OP_RECT, OP_TEXT, OP_ROTATE, OP_RESTORE = 1, 2, 3, 4
WIDTH, FIELD_H = 1080, 260


def font(size, bold=False):
    for path in FALLBACKS:
        try:
            return ImageFont.truetype(path.replace("Sans.ttf", "Sans-Bold.ttf") if bold else path, size)
        except OSError:
            try:
                return ImageFont.truetype(path, size)
            except OSError:
                continue
    return ImageFont.load_default()


def argb(c):
    c &= 0xFFFFFFFF
    return ((c >> 16) & 255, (c >> 8) & 255, c & 255, (c >> 24) & 255)


field, height, background, ops, texts = "", 700, 0xFF202124, [], []
for line in open(sys.argv[1], encoding="utf-8"):
    kind, _, rest = line.rstrip("\n").partition("\t")
    if kind == "field":
        field = rest
    elif kind == "height":
        height = int(rest)
    elif kind == "background":
        background = int(rest)
    elif kind == "op":
        ops.append([int(x) for x in rest.split()])
    elif kind == "text":
        texts.append(rest)

img = Image.new("RGBA", (WIDTH, FIELD_H + height), (255, 255, 255, 255))
d = ImageDraw.Draw(img)
# The field: the text wrapped, the composing word underlined.
f = font(44)
x, y = 30, 30
for ch in field:
    if ch in "[]":
        continue
    w = d.textlength(ch, font=f)
    if x + w > WIDTH - 30:
        x, y = 30, y + 58
    d.text((x, y), ch, font=f, fill=(20, 20, 20, 255) if ch != "│" else (26, 115, 232, 255))
    x += w
inside = False
x, y = 30, 30
for ch in field:
    if ch == "[":
        inside = True
        continue
    if ch == "]":
        inside = False
        continue
    w = d.textlength(ch, font=f)
    if x + w > WIDTH - 30:
        x, y = 30, y + 58
    if inside and ch != "│":
        d.line([(x, y + 52), (x + w, y + 52)], fill=(20, 20, 20, 255), width=3)
    x += w
d.line([(0, FIELD_H - 1), (WIDTH, FIELD_H - 1)], fill=(200, 200, 200, 255), width=2)
# The keyboard.
kb = Image.new("RGBA", (WIDTH, height), argb(background))
k = ImageDraw.Draw(kb)
for op in ops:
    if op[0] == OP_RECT:
        _, x, y, w, h, color, r = op
        k.rounded_rectangle([x, y, x + w, y + h], radius=r, fill=argb(color))
    elif op[0] == OP_TEXT:
        _, cx, base, size, color, idx, bold = op
        t = texts[idx] if idx < len(texts) else ""
        ft = font(max(size, 8), bool(bold))
        w = k.textlength(t, font=ft)
        k.text((cx - w / 2, base), t, font=ft, fill=argb(color), anchor="ls")
img.alpha_composite(kb, (0, FIELD_H))
img.convert("RGB").save(sys.argv[2])
