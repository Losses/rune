// Made by tools/make_glyphs.py: the words on the signs (one column of the atlas each) and how
// many characters each has.

const WORD_COUNT: f32 = 41.0;
const SIGN_WORDS: f32 = 32.0;
const WORD_ROWS: f32 = 4.0;
const GLYPH_SPREAD: f32 = 8.0;
const GLYPH_CELL: f32 = 64.0;

fn word_length(w: f32) -> f32 {
  var lengths = array<f32, 41>(4.0, 4.0, 3.0, 2.0, 2.0, 1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 2.0, 2.0, 2.0, 3.0, 2.0, 2.0, 2.0, 2.0, 4.0, 2.0, 1.0, 3.0, 3.0, 4.0, 1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 2.0, 2.0, 2.0, 2.0, 3.0, 4.0, 2.0, 3.0);
  return lengths[u32(clamp(w, 0.0, WORD_COUNT - 1.0))];
}
