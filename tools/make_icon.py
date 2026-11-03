"""Draw the app icon, src/update_check/icon.ico: a circular update arrow around a check, in the app's blue.

    uv run --no-project --with pillow python tools/make_icon.py

Each size Windows asks for is drawn on its own, eight times larger and then scaled down, so the small ones
stay crisp. Below 32 pixels the arrow would blur into the check, so those sizes show a bold check alone.
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

ICON = Path(__file__).resolve().parents[1] / "src" / "update_check" / "icon.ico"
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
# The app's accent blues, lighter at the top.
TOP, BOTTOM = (59, 130, 246), (29, 78, 216)
SUPERSAMPLE = 8


def draw(size: int) -> Image.Image:
    s = size * SUPERSAMPLE
    icon = _background(s, margin=0.03 if size <= 24 else 0.0625)
    pen = ImageDraw.Draw(icon)
    if size <= 24:
        _check(pen, s, [(0.29, 0.53), (0.44, 0.68), (0.72, 0.37)], width=0.15)
    else:
        # Thicker lines where a pixel is a large part of a line.
        width = 0.085 if size <= 48 else 0.07
        _arrow(pen, s, width)
        _check(pen, s, [(0.385, 0.505), (0.465, 0.585), (0.62, 0.425)], width)
    return icon.resize((size, size), Image.Resampling.LANCZOS)


def _background(s: int, margin: float) -> Image.Image:
    """A rounded square with the blue fading from top to bottom."""
    gradient = Image.new("RGBA", (s, s))
    pen = ImageDraw.Draw(gradient)
    for y in range(s):
        t = y / (s - 1)
        pen.line([(0, y), (s, y)], fill=(*(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)), 255))
    mask = Image.new("L", (s, s), 0)
    edge = margin * s
    ImageDraw.Draw(mask).rounded_rectangle([edge, edge, s - 1 - edge, s - 1 - edge], radius=0.22 * s, fill=255)
    icon = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    icon.paste(gradient, (0, 0), mask)
    return icon


def _check(pen: ImageDraw.ImageDraw, s: int, points: list[tuple[float, float]], width: float) -> None:
    corners = [(x * s, y * s) for x, y in points]
    pen.line(corners, fill="white", width=round(width * s), joint="curve")
    # Round ends.
    r = width * s / 2
    for x, y in (corners[0], corners[-1]):
        pen.ellipse([x - r, y - r, x + r, y + r], fill="white")


def _arrow(pen: ImageDraw.ImageDraw, s: int, width: float) -> None:
    """A ring running clockwise from the upper right round to the top, with its arrowhead in the gap."""
    c, r, w = s / 2, 0.27 * s, width * s
    start, end = 305, 245
    pen.arc([c - r, c - r, c + r, c + r], start, end, fill="white", width=round(w))
    angle = math.radians(end)
    # The middle of the ring's line, which Pillow draws inwards from the circle.
    x, y = c + (r - w / 2) * math.cos(angle), c + (r - w / 2) * math.sin(angle)
    along, out = (-math.sin(angle), math.cos(angle)), (math.cos(angle), math.sin(angle))
    length, half = 0.12 * s, 0.085 * s
    pen.polygon(
        [
            (x + out[0] * half, y + out[1] * half),
            (x - out[0] * half, y - out[1] * half),
            (x + along[0] * length, y + along[1] * length),
        ],
        fill="white",
    )


if __name__ == "__main__":
    images = [draw(size) for size in SIZES]
    # The largest first; Pillow takes each other size from the image drawn for it.
    images[-1].save(ICON, format="ICO", sizes=[(size, size) for size in SIZES], append_images=images[:-1])
    print(f"Wrote {ICON} ({ICON.stat().st_size // 1024} KB)")
