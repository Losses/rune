"""Make the glyph atlas of Neon Street's signs:

    python3 -I make_glyphs.py <face 0.ttf> <face 1.ttf> ... <face 7.ttf>

Needs Pillow, NumPy and SciPy. The faces, in this order (all SIL Open Font License 1.1, their
licences in ../assets/): Noto Sans JP (at weight 800), Dela Gothic One, Reggae One, Rampart One,
RocknRoll One, Zen Antique, Yusei Magic, DotGothic16. See FACES.

Every character the words use is drawn once per face, CELL x CELL, in a grid of COLUMNS cells
across. The eight faces share the grid: faces 0 to 3 are the red, green, blue and alpha channels
of the top block, faces 4 to 7 those of the block under it. Each channel is a signed distance
field, 0.5 on the outline and more inside, so the shader draws the characters sharp at any size.
A word is up to four glyph numbers (one byte each, 255 for none) in one u32, so the shader finds
its characters with one lookup. The long vowel mark has a second glyph turned upright, for
vertical writing. Writes ../assets/glyphs.png and ../glyphs.wgsl.
"""
import pathlib
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont
from scipy.ndimage import distance_transform_edt

# The faces, by their slot (glyphs.wgsl names them FACE_*): a plain heavy gothic for the
# street's own lettering, then the art faces the shops choose from.
FACES = ["NOTO", "DELA", "REGGAE", "RAMPART", "ROCKNROLL", "ANTIQUE", "MAGIC", "DOT"]

# Words seen on any shopping street, no names of shops or brands. At most 4 characters. The order
# of the first 41 is kept: the shader names some of them by number.
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
# How many of the first WORDS the signs choose from.
SIGN_WORDS = 32
# More words for the tenants' signs, stacked up the fronts.
MORE = [
    "スナック", "クラブ", "麻雀", "鍼灸", "英会話", "不動産", "学習塾", "個室",
    "焼酎", "日本酒", "洋食", "鉄板焼", "もつ鍋", "串カツ", "天ぷら", "和食",
    "ホルモン", "古本", "理容", "銭湯",
]
# Big characters: one or two filling a whole sign, the single ones first.
BIG = [
    "酒", "肉", "湯", "鮨", "麺", "宿", "茶", "唄", "呑", "魚", "鳥", "餃",
    "龍", "華", "夢", "遊", "極", "炎", "福", "祭", "蔵", "舞", "粋", "侍",
    "激安", "名物", "元祖", "本店", "酒処", "大吉",
]
ROWS = 4
CELL = 64
COLUMNS = 16
# Rendered this many times larger, for an accurate distance field.
SCALE = 4
# How many atlas pixels the field reaches either side of the outline.
SPREAD = 8.0
WEIGHT = 800

LONG = "ー"
# The long vowel mark turned upright, as vertical writing has it (a glyph of its own).
LONG_V = "|ー"


def render(font, char):
    size = CELL * SCALE
    img = Image.new("L", (size, size), 0)
    draw = ImageDraw.Draw(img)
    box = draw.textbbox((0, 0), char, font=font, anchor="lt")
    w, h = box[2] - box[0], box[3] - box[1]
    draw.text(((size - w) / 2 - box[0], (size - h) / 2 - box[1]), char, font=font, fill=255, anchor="lt")
    return img


def field(img):
    inside = np.asarray(img) > 127
    signed = distance_transform_edt(inside) - distance_transform_edt(~inside)
    f = np.clip(0.5 + signed / SCALE / (2.0 * SPREAD), 0.0, 1.0)
    # Down to CELL by averaging each SCALE x SCALE block.
    return f.reshape(CELL, SCALE, CELL, SCALE).mean(axis=(1, 3))


def load(path):
    font = ImageFont.truetype(path, int(CELL * SCALE * 0.78))
    try:
        font.set_variation_by_axes([WEIGHT])
    except Exception:
        pass
    return font


def missing(font, char):
    # A character the face lacks is drawn as its .notdef box, as a private-use one is.
    return np.array_equal(np.asarray(render(font, char)), np.asarray(render(font, "\U0010FFFD")))


def main():
    here = pathlib.Path(__file__).resolve().parent.parent
    paths = sys.argv[1:]
    assert len(paths) == len(FACES), f"{len(FACES)} fonts, in the order {FACES}"
    fonts = [load(p) for p in paths]
    words = WORDS + MORE + BIG
    glyphs = []
    for word in words:
        assert len(word) <= ROWS, word
        for char in word:
            if char not in glyphs:
                glyphs.append(char)
    glyphs.append(LONG_V)
    assert len(glyphs) < 255
    rows = (len(glyphs) + COLUMNS - 1) // COLUMNS
    block = rows * CELL
    atlas = np.zeros((2 * block, COLUMNS * CELL, 4), dtype=np.float32)
    for slot, font in enumerate(fonts):
        y0 = (slot // 4) * block
        channel = slot % 4
        for g, char in enumerate(glyphs):
            base = LONG if char == LONG_V else char
            face = font
            if missing(font, base):
                print(f"{FACES[slot]} lacks {base}: drawn in {FACES[0]}")
                face = fonts[0]
            img = render(face, base)
            if char == LONG_V:
                img = img.rotate(-90)
            y = y0 + (g // COLUMNS) * CELL
            x = (g % COLUMNS) * CELL
            atlas[y:y + CELL, x:x + CELL, channel] = field(img)
    rgba = (atlas * 255.0 + 0.5).astype(np.uint8)
    Image.fromarray(rgba, "RGBA").save(here / "assets" / "glyphs.png", optimize=True)

    def code(word):
        ids = [glyphs.index(c) for c in word] + [255] * (ROWS - len(word))
        return ids[0] | ids[1] << 8 | ids[2] << 16 | ids[3] << 24

    codes = ", ".join(f"{code(w)}u" for w in words)
    faces = "\n".join(f"const FACE_{name}: f32 = {float(i)};" for i, name in enumerate(FACES))
    (here / "glyphs.wgsl").write_text(f"""// Made by tools/make_glyphs.py: the words on the signs, each as up to four glyph numbers (a byte
// each, 255 for none), and where the glyphs are in the atlas (a grid GLYPH_COLUMNS across, each
// face a channel of one of two blocks GLYPH_BLOCK cells tall).

const WORD_COUNT: f32 = {float(len(words))};
// Words 0 to SIGN_WORDS - 1 and MORE_FIRST to MORE_FIRST + MORE_WORDS - 1 are for shops' signs,
// BIG_FIRST on the big characters, one or two filling a sign.
const SIGN_WORDS: f32 = {float(SIGN_WORDS)};
const MORE_FIRST: f32 = {float(len(WORDS))};
const MORE_WORDS: f32 = {float(len(MORE))};
const BIG_FIRST: f32 = {float(len(WORDS) + len(MORE))};
const BIG_WORDS: f32 = {float(len(BIG))};
// The first BIG_SINGLE of them are one character, the rest two.
const BIG_SINGLE: f32 = {float(sum(len(w) == 1 for w in BIG))};
const WORD_ROWS: f32 = {float(ROWS)};
const GLYPH_SPREAD: f32 = {SPREAD};
const GLYPH_CELL: f32 = {float(CELL)};
const GLYPH_COLUMNS: u32 = {COLUMNS}u;
const GLYPH_BLOCK: f32 = {float(rows)};
// The long vowel mark, and the same turned upright for vertical writing.
const GLYPH_LONG: u32 = {glyphs.index(LONG)}u;
const GLYPH_LONG_V: u32 = {glyphs.index(LONG_V)}u;
{faces}

// A module constant, not an array in the function: one built at every call cost a fifth of the
// street's time where signs fill the view.
const WORD_CODES = array<u32, {len(words)}>({codes});

fn word_code(w: f32) -> u32 {{
  return WORD_CODES[u32(clamp(w, 0.0, WORD_COUNT - 1.0))];
}}

// How many characters word w has.
fn word_length(w: f32) -> f32 {{
  let c = word_code(w);
  return f32(u32((c & 0xffu) != 0xffu) + u32((c & 0xff00u) != 0xff00u) + u32((c & 0xff0000u) != 0xff0000u) + u32((c >> 24u) != 0xffu));
}}
""")
    print(f"{len(words)} words, {len(glyphs)} glyphs, atlas {atlas.shape[1]}x{atlas.shape[0]}")


if __name__ == "__main__":
    main()
