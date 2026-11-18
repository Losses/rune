fn update() {
  state.cam_pos = vec3f(0.0);
  state.cam_right = vec3f(1.0, 0.0, 0.0);
  state.cam_up = vec3f(0.0, 1.0, 0.0);
  state.cam_fwd = vec3f(0.0, 0.0, 1.0);
  state.focal = 1.0;
  state.tile_n = array<u32, TILES>();

  state.pos[0] = vec4f(0.0, 0.0, 1.3, 0.15);
  state.pos[1] = vec4f(0.3, 0.3, 0.81, 0.2);
  // A larger flake exercises the generic near-camera fallback above the flake's depth cutoff.
  state.pos[2] = vec4f(0.15, 0.05, 1.2, 1.0);
  for (var i = 0u; i < 3u; i++) {
    // Roll and tilt the off-axis flake, leaving the other two face-on.
    let roll = select(0.0, 0.785, i == 1u);
    let tilt = select(0.0, 0.6, i == 1u);
    let x = vec3f(cos(tilt), 0.0, -sin(tilt));
    let y = vec3f(0.0, 1.0, 0.0);
    state.rx[i] = vec4f(x * cos(roll) + y * sin(roll), CUBE * 10.0);
    state.ry[i] = vec4f(-x * sin(roll) + y * cos(roll), 0.0);
    state.rz[i] = vec4f(cross(x, y), 0.0);
    project(i);
  }
}
