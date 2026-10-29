// Made by tools/make_glyphs.py: the words on the signs, each as up to four glyph numbers (a byte
// each, 255 for none), and where the glyphs are in the atlas (a grid GLYPH_COLUMNS across, each
// face a channel of one of two blocks GLYPH_BLOCK cells tall).

const WORD_COUNT: f32 = 91.0;
// Words 0 to SIGN_WORDS - 1 and MORE_FIRST to MORE_FIRST + MORE_WORDS - 1 are for shops' signs,
// BIG_FIRST on the big characters, one or two filling a sign.
const SIGN_WORDS: f32 = 32.0;
const MORE_FIRST: f32 = 41.0;
const MORE_WORDS: f32 = 20.0;
const BIG_FIRST: f32 = 61.0;
const BIG_WORDS: f32 = 30.0;
// The first BIG_SINGLE of them are one character, the rest two.
const BIG_SINGLE: f32 = 24.0;
const WORD_ROWS: f32 = 4.0;
const GLYPH_SPREAD: f32 = 8.0;
const GLYPH_CELL: f32 = 64.0;
const GLYPH_COLUMNS: u32 = 16u;
const GLYPH_BLOCK: f32 = 10.0;
// The long vowel mark, and the same turned upright for vertical writing.
const GLYPH_LONG: u32 = 1u;
const GLYPH_LONG_V: u32 = 150u;
const FACE_NOTO: f32 = 0.0;
const FACE_DELA: f32 = 1.0;
const FACE_REGGAE: f32 = 2.0;
const FACE_RAMPART: f32 = 3.0;
const FACE_ROCKNROLL: f32 = 4.0;
const FACE_ANTIQUE: f32 = 5.0;
const FACE_MAGIC: f32 = 6.0;
const FACE_DOT: f32 = 7.0;

// A module constant, not an array in the function: one built at every call cost a fifth of the
// street's time where signs fill the view.
const WORD_CODES = array<u32, 91>(50462976u, 100990980u, 4278781959u, 4294904586u, 4294905100u, 4294967054u, 4294905871u, 4294906120u, 4294906378u, 4294906899u, 4279632149u, 4279834647u, 4280032026u, 4294909469u, 4294909721u, 4294910240u, 4280558370u, 4294911525u, 4294912039u, 4294912553u, 4294913067u, 808398381u, 4294914609u, 4294967091u, 4281742644u, 4281940023u, 1027357498u, 4294967102u, 4294918207u, 4294918721u, 4294919235u, 4294919749u, 4282992711u, 4294921034u, 4294921241u, 4294921805u, 4294922035u, 4283433728u, 1392595537u, 4294923604u, 4283629910u, 1515804759u, 4284153946u, 4294918492u, 4294925917u, 4284571743u, 4284769122u, 4284966501u, 4294909032u, 4294928650u, 4278727530u, 4294906987u, 4278873452u, 4285558638u, 4285662321u, 4285887603u, 4294906998u, 58217335u, 4294914845u, 4294908793u, 4294933370u, 4294967048u, 4294967051u, 4294967163u, 4294967164u, 4294967165u, 4294967166u, 4294967056u, 4294967167u, 4294967168u, 4294967169u, 4294967058u, 4294967072u, 4294967170u, 4294967071u, 4294967171u, 4294967172u, 4294967173u, 4294967174u, 4294967175u, 4294967176u, 4294967177u, 4294967178u, 4294967179u, 4294967180u, 4294938253u, 4294938767u, 4294939281u, 4294920243u, 4294939400u, 4294940052u);

fn word_code(w: f32) -> u32 {
  return WORD_CODES[u32(clamp(w, 0.0, WORD_COUNT - 1.0))];
}

// How many characters word w has.
fn word_length(w: f32) -> f32 {
  let c = word_code(w);
  return f32(u32((c & 0xffu) != 0xffu) + u32((c & 0xff00u) != 0xff00u) + u32((c & 0xff0000u) != 0xff0000u) + u32((c >> 24u) != 0xffu));
}
