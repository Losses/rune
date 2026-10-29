"""Make the scene's emoji: python make_emoji.py [twemoji | noto-3D | noto-2D] [cache dir]

Needs Pillow, and rsvg-convert (librsvg) for Twemoji. Writes ../assets/emoji.png, 2048 x 2048: 64
emoji in an 8 x 8 grid, 256 pixels a cell, each drawn 224 pixels across in the middle of its cell
so the mips do not bleed one into the next. The colours are premultiplied by alpha, so the shader
lays an emoji over a face as face * (1 - a) + rgb, and its edges do not darken as the mips blur
them.

The sources:
- twemoji (the default): Twemoji, flat vector art, in the maintained fork
  https://github.com/jdecked/twemoji, pinned at 17.0.3. Its graphics are under CC-BY 4.0 (by Twitter,
  Inc and other contributors): credit them. The SVGs are drawn by rsvg-convert at 224 pixels, so
  they are sharp however large the scene shows one.
- noto-3D, noto-2D: Noto Color Emoji (https://github.com/googlefonts/noto-emoji), under the
  Apache License 2.0: the glossy 3D set or the flat 2D one. Its images are 128 pixels, so they are
  drawn blown up to the 224.
They are downloaded once into the cache directory (by default .emoji-cache beside the tools, kept
out of git). Change EMOJI to choose others: each is its codepoints, as the source names its files
(Twemoji drops fe0f).
"""
import pathlib
import subprocess
import sys
import urllib.request

from PIL import Image

EMOJI = [
    # Music.
    "1f3b5", "1f3b6", "1f3a7", "1f3a4", "1f3b8", "1f941", "1f3b9", "1f3ba",
    # Stars, fire and the sky.
    "2b50", "1f31f", "2728", "1f4ab", "1f525", "26a1", "1f308", "1f319",
    # Gems and hearts.
    "1f48e", "2764", "1f49c", "1f499", "1f49a", "1f49b", "1f9e1", "1f496",
    # Faces.
    "1f600", "1f606", "1f60e", "1f973", "1f60d", "1f929", "1f63a", "1f61c",
    # Animals.
    "1f431", "1f436", "1f98a", "1f43c", "1f427", "1f430", "1f984", "1f419",
    # Sweets and fruit.
    "1f353", "1f352", "1f351", "1f369", "1f370", "1f366", "1f36d", "1f349",
    # Play.
    "1f680", "1f6f8", "1f380", "1faa9", "1f3af", "1f3c6", "1f381", "1f389",
    # Nature.
    "1f338", "1f33b", "1f340", "1f30a", "2600", "2744", "1f344", "1f30d",
]
NOTO = "https://raw.githubusercontent.com/googlefonts/noto-emoji/main/{}/png/128/emoji_u{}.png"
TWEMOJI = "https://cdn.jsdelivr.net/gh/jdecked/twemoji@17.0.3/assets/svg/{}.svg"
CELL = 256
DRAWN = 224
GRID = 8


def download(url, path):
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(url, timeout=30) as r:
            path.write_bytes(r.read())
    return path


def fetch(source, code, cache):
    """The emoji's image as a file: a PNG (Twemoji's SVG drawn at DRAWN pixels)."""
    if source == "twemoji":
        svg = download(TWEMOJI.format(code), cache / "twemoji" / f"{code}.svg")
        png = cache / "twemoji" / f"{code}-{DRAWN}.png"
        if not png.exists():
            subprocess.run(
                ["rsvg-convert", "-w", str(DRAWN), "-h", str(DRAWN), "-o", str(png), str(svg)],
                check=True,
            )
        return png
    style = source.removeprefix("noto-")
    return download(NOTO.format(style, code), cache / style / f"emoji_u{code}.png")


def premultiplied(im):
    im = im.convert("RGBA")
    px = im.load()
    for y in range(im.height):
        for x in range(im.width):
            r, g, b, a = px[x, y]
            px[x, y] = (r * a // 255, g * a // 255, b * a // 255, a)
    return im


def main():
    here = pathlib.Path(__file__).resolve().parent
    source = sys.argv[1] if len(sys.argv) > 1 else "twemoji"
    assert source in ("twemoji", "noto-3D", "noto-2D")
    cache = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else here.parent / ".emoji-cache"
    cache.mkdir(parents=True, exist_ok=True)
    assert len(EMOJI) == GRID * GRID
    atlas = Image.new("RGBA", (CELL * GRID, CELL * GRID), (0, 0, 0, 0))
    for i, code in enumerate(EMOJI):
        im = Image.open(fetch(source, code, cache)).convert("RGBA")
        im.thumbnail((DRAWN, DRAWN), getattr(Image, "Resampling", Image).LANCZOS)
        cx = (i % GRID) * CELL + (CELL - im.width) // 2
        cy = (i // GRID) * CELL + (CELL - im.height) // 2
        atlas.paste(premultiplied(im), (cx, cy))
    out = here.parent / "assets" / "emoji.png"
    out.parent.mkdir(exist_ok=True)
    atlas.save(out)
    print(out)


if __name__ == "__main__":
    main()
