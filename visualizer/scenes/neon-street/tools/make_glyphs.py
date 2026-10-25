"""Make the glyph atlas of Neon Street's signs: python make_glyphs.py <font.ttf>

Needs Pillow, NumPy and SciPy. The font is Noto Sans JP (SIL Open Font License 1.1).

Each word of WORDS is one column of the atlas, one character per CELL x CELL cell from the top:
the top ROWS rows are written for vertical signs (the long vowel mark turned upright), the
ROWS below for horizontal ones.
The red channel is a signed distance field, 0.5 on the outline and more inside, so the shader
draws the characters sharp at any size. Writes ../assets/glyphs.png and ../glyphs.wgsl.
"""
import pathlib
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy.ndimage import distance_transform_edt

# Words seen on any shopping street, no names of shops or brands. At most ROWS characters.
WORDS = [
    "ラーメン", "カラオケ", "居酒屋", "焼肉", "寿司", "薬", "喫茶", "酒場",
    "焼鳥", "定食", "ゲーム", "営業中", "美容室", "古着", "中華", "餃子",
    "うどん", "そば", "歯科", "占い", "珈琲", "立ち飲み", "漫画", "本",
    "BAR", "24H", "OPEN", "質", "整体", "雀荘", "弁当", "牛丼",
    # Not for the signs: the arch over the shopping street (a name, then the word for shopping
    # street) and the banners.
    "商店街", "銀座", "中央", "駅前", "本町",
    "ランチ", "生ビール", "特売", "セール",
]
# How many of WORDS the signs choose from.
SIGN_WORDS = 32
ROWS = 4
CELL = 64
# Rendered this many times larger, for an accurate distance field.
SCALE = 4
# How many atlas pixels the field reaches either side of the outline.
SPREAD = 8.0
WEIGHT = 800


# Written upright in vertical text.
TURNED = set("ー〜～")


def glyph_field(font, char, vertical):
    size = CELL * SCALE
    img = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(img)
    box = draw.textbbox((0, 0), char, font=font, anchor="lt")
    w, h = box[2] - box[0], box[3] - box[1]
    draw.text(((size - w) / 2 - box[0], (size - h) / 2 - box[1]), char, font=font, fill=255, anchor="lt")
    if vertical and char in TURNED:
        img = img.rotate(-90)
    inside = np.asarray(img) > 127
    signed = distance_transform_edt(inside) - distance_transform_edt(~inside)
    field = np.clip(0.5 + signed / SCALE / (2.0 * SPREAD), 0.0, 1.0)
    # Down to CELL by averaging each SCALE x SCALE block.
    return field.reshape(CELL, SCALE, CELL, SCALE).mean(axis=(1, 3))


def main():
    here = pathlib.Path(__file__).resolve().parent.parent
    font = ImageFont.truetype(sys.argv[1], int(CELL * SCALE * 0.78))
    try:
        font.set_variation_by_axes([WEIGHT])
    except Exception:
        pass
    atlas = np.zeros((2 * ROWS * CELL, len(WORDS) * CELL), dtype=np.float32)
    for column, word in enumerate(WORDS):
        assert len(word) <= ROWS, word
        for row, char in enumerate(word):
            for half in range(2):
                y = (half * ROWS + row) * CELL
                atlas[y:y + CELL, column * CELL:(column + 1) * CELL] = glyph_field(font, char, half == 0)
    red = (atlas * 255.0 + 0.5).astype(np.uint8)
    rgba = np.stack([red, red, red, np.full_like(red, 255)], axis=-1)
    Image.fromarray(rgba, "RGBA").save(here / "assets" / "glyphs.png", optimize=True)
    lengths = ", ".join(f"{float(len(w))}" for w in WORDS)
    (here / "glyphs.wgsl").write_text(f"""// Made by tools/make_glyphs.py: the words on the signs (one column of the atlas each) and how
// many characters each has.

const WORD_COUNT: f32 = {float(len(WORDS))};
const SIGN_WORDS: f32 = {float(SIGN_WORDS)};
const WORD_ROWS: f32 = {float(ROWS)};
const GLYPH_SPREAD: f32 = {SPREAD};
const GLYPH_CELL: f32 = {float(CELL)};

fn word_length(w: f32) -> f32 {{
  var lengths = array<f32, {len(WORDS)}>({lengths});
  return lengths[u32(clamp(w, 0.0, WORD_COUNT - 1.0))];
}}
""")
    print(f"{len(WORDS)} words, atlas {atlas.shape[1]}x{atlas.shape[0]}")


if __name__ == "__main__":
    main()
