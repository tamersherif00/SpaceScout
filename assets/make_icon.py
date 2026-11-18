"""Generates assets/spacescout.ico and assets/icon-256.png.

Run: python assets/make_icon.py   (requires Pillow)
Each size is drawn at 8x and downsampled so small sizes stay crisp.
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).parent
SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]
SS = 8  # supersampling factor


def lerp(a, b, t):
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def draw(size: int) -> Image.Image:
    s = size * SS
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))

    # Rounded-square background with a vertical blue gradient.
    grad = Image.new("RGBA", (s, s))
    gd = ImageDraw.Draw(grad)
    top, bottom = (59, 130, 246), (30, 64, 175)
    for y in range(s):
        gd.line([(0, y), (s, y)], fill=lerp(top, bottom, y / (s - 1)) + (255,))
    mask = Image.new("L", (s, s), 0)
    inset = round(s * 0.04)
    ImageDraw.Draw(mask).rounded_rectangle(
        [inset, inset, s - inset - 1, s - inset - 1], radius=round(s * 0.22), fill=255
    )
    img.paste(grad, (0, 0), mask)

    d = ImageDraw.Draw(img)
    white = (255, 255, 255, 255)

    # Magnifier: lens centre and radius, as fractions of the canvas.
    cx, cy, r = 0.43 * s, 0.43 * s, 0.25 * s
    ring = max(round(s * 0.075), SS * 2)

    # Handle, drawn first so the lens ring sits on top of its end.
    hx0, hy0 = cx + r * 0.62, cy + r * 0.62
    hx1, hy1 = 0.80 * s, 0.80 * s
    hw = max(round(s * 0.11), SS * 2)
    d.line([(hx0, hy0), (hx1, hy1)], fill=white, width=hw)
    d.ellipse([hx1 - hw / 2, hy1 - hw / 2, hx1 + hw / 2, hy1 + hw / 2], fill=white)

    # Lens interior: dark glass with three size bars (largest first).
    inner = r - ring / 2
    d.ellipse([cx - inner, cy - inner, cx + inner, cy + inner], fill=(15, 23, 42, 255))
    bars = [((251, 191, 36), 0.95), ((52, 211, 153), 0.62), ((147, 197, 253), 0.38)]
    bw = inner * 0.42
    gap = inner * 0.12
    total = 3 * bw + 2 * gap
    x = cx - total / 2
    base = cy + inner * 0.55
    for colour, h in bars:
        d.rectangle([x, base - inner * 1.15 * h, x + bw, base], fill=colour + (255,))
        x += bw + gap
    # Lens ring on top (the bars fit inside the glass at these proportions).
    d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=white, width=ring)

    return img.resize((size, size), Image.LANCZOS)


def main():
    frames = [draw(n) for n in SIZES]
    frames[-1].save(OUT / "icon-256.png")
    frames[-1].save(OUT / "spacescout.ico", sizes=[(n, n) for n in SIZES], append_images=frames[:-1])
    print("wrote", OUT / "spacescout.ico", "and", OUT / "icon-256.png")


if __name__ == "__main__":
    main()
