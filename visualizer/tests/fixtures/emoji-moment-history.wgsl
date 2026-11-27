// Which witness answers which moment, in place of the scene's hash. That hash amplifies the last
// bit of its arithmetic, which differs from one GPU to the next, so no emoji answers the same
// moments everywhere. `moment_for` asks with the emoji's id plus (1.618 n, 3.1) for moment n; the
// witnesses' ids are 1000 apart.
fn hash(p: vec2f) -> f32 {
  let witness = u32(p.x / 1000.0);
  let n = u32(round((p.x - 1000.0 * f32(witness)) / 1.618));
  // The first emoji answers note 2 and no later event. The second also answers the
  // final note, but not the concurrent hit. The third answers both concurrent events.
  let answers = (n == 2u && witness < 2u) || (n == 19u && witness >= 1u) || (n == 20u && witness == 2u);
  return select(1.0, 0.0, answers);
}

fn update() {
  tunnel_update();
  state.witness[0] = moment_for(vec2f(0.0, 17.0));
  state.witness[1] = moment_for(vec2f(1000.0, 17.0));
  state.witness[2] = moment_for(vec2f(2000.0, 17.0));
}

fn shade(px: Pixel) -> vec4f {
  let moment = state.witness[min(u32(px.uv.x * 3.0), 2u)];
  return vec4f(min(moment.x, 2.0) * 0.5, moment.y * 0.5, 0.0, 1.0);
}
