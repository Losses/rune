"""Make the badges' emoji: python make_emoji.py [3D | 2D] [cache dir]

Needs Pillow. Writes ../assets/emoji.png, 1024 x 1024: 64 emoji in an 8 x 8 grid, 128 pixels a
cell, each drawn 112 pixels across in the middle of its cell so the mips do not bleed one into the
next. The colours are premultiplied by alpha, so the shader lays an emoji over a face as
face * (1 - a) + rgb, and its edges do not darken as the mips blur them.

The emoji are Noto Color Emoji (https://github.com/googlefonts/noto-emoji), whose images are
under the Apache License 2.0: the glossy 3D set by default, which suits the shiny badges, or the
flat 2D one. They are downloaded once into the cache directory (by default .emoji-cache beside
the tools, kept out of git). Change EMOJI to choose others: each is its codepoints, as Noto names
its files.
"""
import pathlib
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
    "1f680", "1f6f8", "1f3ae", "1f47e", "1f3af", "1f3c6", "1f381", "1f388",
    # Nature.
    "1f338", "1f33b", "1f340", "1f30a", "2600", "2744", "1f344", "1f30d",
]
URL = "https://raw.githubusercontent.com/googlefonts/noto-emoji/main/{}/png/128/emoji_u{}.png"
CELL = 128
DRAWN = 112
GRID = 8


def fetch(style, code, cache):
    path = cache / style / f"emoji_u{code}.png"
    if not path.exists():
        path.parent.mkdir(parents=True, exist_ok=True)
        with urllib.request.urlopen(URL.format(style, code), timeout=30) as r:
            path.write_bytes(r.read())
    return path


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
    style = sys.argv[1] if len(sys.argv) > 1 else "3D"
    assert style in ("3D", "2D")
    cache = pathlib.Path(sys.argv[2]) if len(sys.argv) > 2 else here.parent / ".emoji-cache"
    cache.mkdir(parents=True, exist_ok=True)
    assert len(EMOJI) == GRID * GRID
    atlas = Image.new("RGBA", (CELL * GRID, CELL * GRID), (0, 0, 0, 0))
    for i, code in enumerate(EMOJI):
        im = Image.open(fetch(style, code, cache)).convert("RGBA")
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
