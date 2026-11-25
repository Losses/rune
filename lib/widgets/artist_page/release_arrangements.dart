import 'dart:math';

import '../tile/rolling_cover_circles.dart';

/// How the circles are laid out on each release tile of one page: picked at
/// random when made, with as few small tiles alike as the arrangements allow,
/// and the same for as long as it lives.
class ReleaseArrangements {
  /// Every arrangement, in the order this page uses them.
  final List<CircleArrangementKind> _order;

  ReleaseArrangements([Random? random])
    : _order = [...CircleArrangementKind.values]..shuffle(random ?? Random());

  // The circles each arrangement has in sight at a size, which depends on
  // nothing else and is not worked out again for every tile of every build.
  final _inSight = <(CircleArrangementKind, int), int>{};

  int _circlesInSight(CircleArrangementKind kind, int cells) =>
      _inSight[(kind, cells)] ??= CircleArrangement.of(kind, cells).inSight;

  /// The arrangement of the tile of the section at [index], a tile of [cells]
  /// cells a side whose section has [covers] covers, none counted twice.
  ///
  /// The tiles of one cell go through their arrangements in turn, so none
  /// comes again before all have been used. A larger tile, which only the
  /// first section has, takes any of the arrangements, which may be one a
  /// smaller tile also has: at another size it looks unlike it. A tile keeps
  /// its arrangement when another changes its size.
  ///
  /// A tile only takes an arrangement whose circles in sight its covers can
  /// fill, or else one of those with the fewest circles in sight, and goes
  /// through these alone.
  CircleArrangementKind of(int index, int cells, int covers) {
    final large = cells > 1;
    final inSight = {
      for (final kind in _order)
        if (large || kind != CircleArrangementKind.bigAndSmall)
          kind: _circlesInSight(kind, cells),
    };
    final most = max(covers, inSight.values.reduce(min));
    final kinds = [
      for (final MapEntry(key: kind, value: circles) in inSight.entries)
        if (circles <= most) kind,
    ];
    return large ? kinds.first : kinds[index % kinds.length];
  }
}
