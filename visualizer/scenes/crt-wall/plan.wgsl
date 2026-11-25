// The plan, once a frame (see the plan in wall.wgsl): a texel each of what it holds.

fn shade(px: Pixel) -> vec4f {
  let at = vec2u(px.coord);
  if (at.x >= PLAN_WIDTH || at.y >= PLAN_HEIGHT) {
    return vec4f(0.0);
  }
  if (at.y < PLAN_ROWS) {
    // A block of a bay: its key, the bay or a column's pile.
    let key = plan_key(at.y);
    if (at.x == 0u) {
      return vec4f(key, 1.0, 0.0);
    }
    let bay = bay_of(key.x, key.y);
    if (at.x < 4u) {
      return bay_texel(bay, at.x - 1u);
    }
    let j = (at.x - 4u) / 5u;
    if (j >= bay.count) {
      return vec4f(0.0);
    }
    return pile_texel(pile_of(column_j(bay, key.x, key.y, j)), (at.x - 4u) % 5u);
  }
  if (at.y < PLAN_SCOPE) {
    // The pools' cells, and where they and the bays' window start.
    let first = plan_first_pool();
    if (at.x < PLAN_POOLS) {
      return vec4f(pool_of(first + vec2f(f32(at.x), f32(at.y - PLAN_ROWS))), 1.0);
    }
    if (at.y == PLAN_ROWS && at.x == PLAN_POOLS) {
      return vec4f(first, 1.0, 0.0);
    }
    if (at.y == PLAN_ROWS && at.x == PLAN_POOLS + 1u) {
      return vec4f(plan_corner(), 1.0, 0.0);
    }
    return vec4f(0.0);
  }
  if (at.y < PLAN_SCOPE + 2u) {
    // Two points of the stereo picture.
    if (at.x >= 24u) {
      return vec4f(0.0);
    }
    let i = i32((at.y - PLAN_SCOPE) * 48u + at.x * 2u);
    return vec4f(stereo_point(i), stereo_point(i + 1));
  }
  // The box around a run of segments.
  if (at.x >= u32(SCOPE_RUNS)) {
    return vec4f(0.0);
  }
  let first = i32(at.x) * 8;
  var low = stereo_point(first);
  var high = low;
  for (var i = first + 1; i <= min(first + 8, POINTS - 1); i++) {
    let p = stereo_point(i);
    low = min(low, p);
    high = max(high, p);
  }
  return vec4f(low, high);
}
