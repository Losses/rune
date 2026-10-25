// What Badge Tunnel keeps from frame to frame: how far the camera has flown down the tunnel and
// how fast, the music it flies to, the shot; and the things ahead of it, worked out afresh each
// frame (badges, cubes and sparkles), already projected and sorted into screen tiles, so a pixel
// looks only at the few that could cover it.

// The tunnel is cut into slots along its length, each holding up to PER things, and the camera
// sees SLOTS of them, ahead or (looking back) behind.
const SLOTS: u32 = 64u;
const PER: u32 = 3u;
const ITEMS: u32 = 192u;
const SLOT_LEN: f32 = 1.6;
// The screen's tiles, and how many things a tile lists at most (nearest first).
const TX: u32 = 16u;
const TY: u32 = 9u;
const TILES: u32 = 144u;
const LIST: u32 = 32u;
// The kinds of thing (a cube is a flake of gold or silver leaf; an emoji flies on its own, cut
// to its outline, where the content is emoji rather than badges).
const BADGE: f32 = 0.0;
const CUBE: f32 = 1.0;
const SPARK: f32 = 2.0;
const EMOJI: f32 = 3.0;
// How many crests assets/crests.png holds.
const CRESTS: u32 = 12u;

// Where the camera films from: turned aside (yaw), up (pitch) and rolled, and how long its lens
// is (times `focal`); and how far it stands off the tunnel's middle, across and up.
struct Pose {
  turn: vec4f,
  at: vec4f,
}

struct State {
  // How far the camera has come, how fast it flies, and the surge a kick gives it.
  s: f32,
  speed: f32,
  surge: f32,
  // The kick (1 on it, falling away), the loudness now (smoothed), and the badges' turn: a step on
  // every beat, eased.
  kick: f32,
  moment: f32,
  turn: f32,
  turn_to: f32,
  // The place the camera is filming from or gliding to (see update), and its side; when it set
  // off, from where, and how long the glide takes; the bars since. The badges' colours from then
  // on (0 black lacquer and gold, 1 vermilion and gold, 2 indigo and silver).
  view: f32,
  side: f32,
  last_view: f32,
  set_off: Pose,
  glide: f32,
  bars_on: f32,
  palette: f32,
  // The camera.
  cam_pos: vec3f,
  cam_fwd: vec3f,
  cam_right: vec3f,
  cam_up: vec3f,
  focal: f32,
  // Each thing: its centre and size (w); the rows of its turn from the world into its own frame,
  // their w its kind and shape, its look (lacquer and leaf) and its crest; and where it is on the
  // screen (centre, radius, and its distance along the view).
  pos: array<vec4f, 192>,
  rx: array<vec4f, 192>,
  ry: array<vec4f, 192>,
  rz: array<vec4f, 192>,
  scr: array<vec4f, 192>,
  // The melody: its last eight notes, in turn (when, how strong), how many have been heard, the
  // chart's melody onset last frame, and (without a chart) the mids' slow average and whether a
  // note may come (the mids having fallen back since the last).
  notes: array<vec4f, 8>,
  note_count: f32,
  heard_note: f32,
  mids_slow: f32,
  note_ready: f32,
  // How many emoji the music calls for: the notes a second of late (smoothed), how loud the music
  // has been of late (without a chart), the farthest slot given its share yet, and each slot's
  // share and badge colours, set as it comes into sight far ahead (so what is near never comes and
  // goes, nor changes colour): kept for as far behind as ahead, for a camera looking back.
  note_rate: f32,
  loud_slow: f32,
  last_slot: f32,
  slot_set: array<vec2f, 128>,
  // The colours, all from the player's theme colour if it follows it (see update).
  colours: array<vec4f, 24>,
  // The tiles: how many each lists, and the lists.
  tile_n: array<u32, 144>,
  tile: array<u32, 4608>,
}

fn hash(p: vec2f) -> f32 {
  var p3 = fract(vec3f(p.x, p.y, p.x) * 0.1031);
  p3 += dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

// Where a point on the screen lies in uv (0..1, y down) from its place in Pixel.p (centred, y up,
// by height).
fn screen_uv(p: vec2f, aspect: f32) -> vec2f {
  return vec2f(p.x / aspect + 0.5, 0.5 - p.y);
}
