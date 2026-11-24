fn shade(px: Pixel) -> vec4f {
  // Pick the test flake by region; the rays and tile coordinates remain those of the real pixel.
  let i = select(select(0u, 1u, px.p.x > 0.5), 2u, px.p.x < -0.25);
  let d = normalize(vec3f(px.p, state.focal));
  let pos = state.pos[i];
  let basis = transpose(mat3x3f(state.rx[i].xyz, state.ry[i].xyz, state.rz[i].xyz));
  let hit = flake_hit(basis * -pos.xyz / pos.w, basis * d / pos.w).x < 1e8;

  let sc = state.scr[i];
  let off = px.p - sc.xy;
  let tile = min(u32(px.uv.y * f32(TY)), TY - 1u) * TX + min(u32(px.uv.x * f32(TX)), TX - 1u);
  var listed = false;
  for (var j = 0u; j < state.tile_n[tile]; j++) {
    listed = listed || state.tile[tile * LIST + j] == i;
  }
  return vec4f(f32(hit), f32(dot(off, off) <= sc.z * sc.z), f32(listed), 1.0);
}
