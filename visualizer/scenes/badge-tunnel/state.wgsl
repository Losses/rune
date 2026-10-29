// What Badge Tunnel keeps from frame to frame: how far the camera has flown down the tunnel and
// how fast, the scene's own time and the hits it has counted, the shot; and the things ahead of
// it, worked out afresh each frame (badges, cubes and sparkles), already projected and sorted into
// screen tiles, so a pixel looks only at the few that could cover it.

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
  // How far the camera has come and how fast it flies.
  s: f32,
  speed: f32,
  // The scene's own time for everything that moves of itself (the emoji's and badges' turning,
  // the flakes' tumbling, the core's rays, the sparkles' twinkle, the camera's drift): it
  // advances by `dt` times how much is heard (`input.audible`), so all of it slows with a fade
  // and stands still in silence. (`frame.time` stays where it measures the time since an event:
  // a note's move, a glide.)
  phase: f32,
  // The hits the scene counts (see update): how many, the seconds since the last, and what they
  // give: the kick (a decaying envelope, rocking the emoji), the surge forward, and the crests'
  // turn (the count, the last step eased in over a moment).
  count: f32,
  step_age: f32,
  kick: f32,
  surge: f32,
  turn: f32,
  // The place the camera is filming from or gliding to (see update), its side and its dutch
  // angle; how many of the director's cuts it has followed; when it set off, from where, and how
  // long the glide takes. (The badges' colours from then on are `input.coming`: 0 black
  // lacquer and gold, 1 vermilion and gold, 2 indigo and silver.)
  view: f32,
  side: f32,
  roll_to: f32,
  cuts: u32,
  last_view: f32,
  set_off: Pose,
  glide: f32,
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
  // The melody: its last eight notes, in turn (when, how strong), and how many have been heard.
  notes: array<vec4f, 8>,
  note_count: f32,
  // How many emoji the music calls for: the notes a second of late (smoothed), the farthest slot
  // given its share yet, and each slot's
  // share and badge colours, set as it comes into sight far ahead (so what is near never comes and
  // goes, nor changes colour): kept for as far behind as ahead, for a camera looking back.
  note_rate: f32,
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
