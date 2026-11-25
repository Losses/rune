fn update() {
  tunnel_update();
  // The first emoji answers note 2 and no later event. The second also answers the
  // final note, but not the concurrent hit. The third answers both concurrent events.
  state.witness[0] = moment_for(vec2f(349.0, 17.0));
  state.witness[1] = moment_for(vec2f(1079.0, 17.0));
  state.witness[2] = moment_for(vec2f(88.0, 17.0));
}

fn shade(px: Pixel) -> vec4f {
  let moment = state.witness[min(u32(px.uv.x * 3.0), 2u)];
  return vec4f(min(moment.x, 2.0) * 0.5, moment.y * 0.5, 0.0, 1.0);
}
