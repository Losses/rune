import 'dart:async';
import 'dart:math';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show listEquals, mergeSort;
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/draw_image_to_canvas.dart';

import 'constants/image_memory_manager.dart';
import 'utils/image_proxy.dart';

/// The shortest and the longest rest between two rolls of a tile.
const rollingRestMin = Duration(seconds: 6);
const rollingRestMax = Duration(seconds: 11);

/// How long a roll of one slot takes, and a roll of two.
const rollingOneSlot = Duration(milliseconds: 900);
const rollingTwoSlots = Duration(milliseconds: 1300);

/// The easing of a roll.
const rollingCurve = Cubic(0.45, 0.05, 0.25, 1);

/// How long a circle that changes its cover in place takes to fade out, and
/// to fade in again.
const rollingFade = Duration(milliseconds: 450);

/// How often a turn fades such a circle instead of rolling a lane.
const rollingFadeShare = 0.3;

/// How long a cover that failed to load waits before it is asked for again.
const rollingCoverRetry = Duration(seconds: 30);

/// The ways the circles of a tile are laid out.
enum CircleArrangementKind {
  /// Rows and columns, which are the lanes.
  square,

  /// Rows set off by half a circle. The rows and both diagonals are lanes.
  hex,

  /// The square lattice turned by an eighth, with its diagonals as lanes.
  diamond,

  /// One circle of twice the size, which changes its cover in place, with a
  /// column and a row of small ones as lanes. Only for a tile of two by two
  /// cells.
  bigAndSmall,
}

/// A place for one circle on a tile. [center] and [diameter] are in parts of
/// the tile's side. A slot that is not [inSight] lies off the tile, and holds
/// a circle that is about to enter or has just left.
@visibleForTesting
@immutable
class CircleSlot {
  final Offset center;
  final double diameter;
  final bool inSight;

  const CircleSlot({
    required this.center,
    required this.diameter,
    required this.inSight,
  });
}

/// Where the circles of a tile are and how they move: the [slots], and the
/// [lanes], each the slots a roll shifts circles along, in order. A lane
/// starts and ends with [hiddenEnds] hidden slots, so a roll moves at most
/// that many steps. A slot may be on several lanes, or on none, and then its
/// circle never rolls. The circles of [fades] change their cover in place.
@visibleForTesting
@immutable
class CircleArrangement {
  final List<CircleSlot> slots;
  final List<List<int>> lanes;
  final int hiddenEnds;
  final List<int> fades;

  const CircleArrangement({
    required this.slots,
    required this.lanes,
    required this.hiddenEnds,
    this.fades = const [],
  });

  /// The arrangement of [kind] for a tile of [cells] cells a side. A tile of
  /// one cell has no [CircleArrangementKind.bigAndSmall], and is given the
  /// square one for it.
  factory CircleArrangement.of(CircleArrangementKind kind, int cells) {
    final large = cells > 1;
    return switch (kind) {
      CircleArrangementKind.square => CircleArrangement.square(large ? 3 : 2),
      CircleArrangementKind.hex => CircleArrangement.hex(large ? 3.2 : 2.2),
      CircleArrangementKind.diamond => CircleArrangement.diamond(
        large ? 2.4 : 1.6,
      ),
      CircleArrangementKind.bigAndSmall =>
        large ? CircleArrangement.bigAndSmall() : CircleArrangement.square(2),
    };
  }

  /// Rows of touching circles, each set off from the last by half a circle,
  /// with [across] circles to the tile's side. The rows and the two
  /// diagonals are the lanes.
  factory CircleArrangement.hex(double across) {
    final d = 1 / across;
    final rise = d * sqrt(3) / 2;
    return CircleArrangement._lattice(
      d,
      (q, r) => Offset(d * 0.3 + d * (q + r / 2), d * 0.45 + rise * r),
      [(a, b) => (b, a), (a, b) => (a, b), (a, b) => (-a - b, b)],
    );
  }

  /// The square lattice turned by an eighth about the tile's middle, with
  /// [across] circles to the tile's side. Circles touch along the two
  /// diagonals, which are the lanes.
  factory CircleArrangement.diamond(double across) {
    final step = 1 / across / sqrt2;
    return CircleArrangement._lattice(
      1 / across,
      (i, j) => Offset(0.5 + (i - j) * step, 0.5 + (i + j) * step),
      [(a, b) => (a, b), (a, b) => (b, a)],
    );
  }

  /// A circle of twice the size at the top left, a column of small circles
  /// at the right and a row of them at the bottom, which ends in the
  /// column's last circle in sight. The column and the row are the lanes,
  /// and the large circle changes its cover in place.
  factory CircleArrangement.bigAndSmall() {
    const hidden = 2;
    const bleed = 0.2;
    const small = 1 / (3 - bleed * 2);
    Offset at(double column, double row) =>
        Offset((column - bleed + 0.5) * small, (row - bleed + 0.5) * small);

    final slots = [
      CircleSlot(center: at(0.5, 0.5), diameter: small * 2, inSight: true),
    ];
    int add(Offset center) {
      slots.add(_slot(center, small));
      return slots.length - 1;
    }

    final column = [
      for (var row = -hidden; row < 3 + hidden; row += 1) add(at(2, row * 1.0)),
    ];
    final row = [
      for (var c = -hidden; c < 2; c += 1) add(at(c * 1.0, 2)),
      column[hidden + 2],
      for (var c = 3; c < 3 + hidden; c += 1) add(at(c * 1.0, 2)),
    ];
    return CircleArrangement(
      slots: slots,
      lanes: [column, row],
      hiddenEnds: hidden,
      fades: const [0],
    );
  }

  /// A slot at [center], in sight when its circle reaches into the tile.
  static CircleSlot _slot(Offset center, double diameter) {
    final outside = Offset(
      max(max(-center.dx, center.dx - 1), 0),
      max(max(-center.dy, center.dy - 1), 0),
    );
    return CircleSlot(
      center: center,
      diameter: diameter,
      inSight: outside.distance < diameter / 2 - 1e-9,
    );
  }

  /// Circles of the diameter [d] on the points [at] of a lattice. Each of
  /// [directions] gives a point the lane it is on and its place along that
  /// lane. A lane keeps the circles it has in sight and two hidden ones at
  /// each end, and the points on no lane are left out.
  factory CircleArrangement._lattice(
    double d,
    Offset Function(int a, int b) at,
    List<(int lane, int place) Function(int a, int b)> directions,
  ) {
    const hidden = 2;
    // Far enough for every lane in sight to reach its hidden ends.
    final margin = d * (hidden + 0.6);
    final reach = (1 / d).ceil() * 2 + hidden * 2 + 2;
    final points = <(int, int, CircleSlot)>[];
    for (var a = -reach; a <= reach; a += 1) {
      for (var b = -reach; b <= reach; b += 1) {
        final center = at(a, b);
        if (center.dx < -margin ||
            center.dx > 1 + margin ||
            center.dy < -margin ||
            center.dy > 1 + margin) {
          continue;
        }
        points.add((a, b, _slot(center, d)));
      }
    }

    final lanes = <List<int>>[];
    for (final direction in directions) {
      final groups = <int, List<(int, int)>>{};
      for (final (index, (a, b, _)) in points.indexed) {
        final (lane, place) = direction(a, b);
        groups.putIfAbsent(lane, () => []).add((place, index));
      }
      for (final group in groups.values) {
        group.sort((x, y) => x.$1.compareTo(y.$1));
        final lane = [for (final (_, index) in group) index];
        final first = lane.indexWhere((i) => points[i].$3.inSight);
        final last = lane.lastIndexWhere((i) => points[i].$3.inSight);
        if (first < hidden || last + hidden >= lane.length) continue;
        lanes.add(lane.sublist(first - hidden, last + hidden + 1));
      }
    }

    // Only the points on a lane become slots.
    final kept = lanes.expand((lane) => lane).toSet().toList()..sort();
    final slotOf = {for (final (slot, index) in kept.indexed) index: slot};
    return CircleArrangement(
      slots: [for (final index in kept) points[index].$3],
      lanes: [
        for (final lane in lanes) [for (final index in lane) slotOf[index]!],
      ],
      hiddenEnds: hidden,
    );
  }

  /// [count] by [count] circles in sight on a square lattice, each touching
  /// its neighbours, with the outer ones reaching past the tile's edges. The
  /// rows and the columns are the lanes.
  factory CircleArrangement.square(int count) {
    const hidden = 2;
    // The part of a circle that the outer circles reach past the edge by.
    const bleed = 0.2;
    final side = count + hidden * 2;
    final slot = 1 / (count - bleed * 2);
    bool isHidden(int i) => i < hidden || i >= hidden + count;

    final slots = <CircleSlot>[];
    final indexes = <(int, int), int>{};
    for (var row = 0; row < side; row += 1) {
      for (var column = 0; column < side; column += 1) {
        // A corner is on no lane in sight, so it has no slot.
        if (isHidden(row) && isHidden(column)) continue;
        indexes[(row, column)] = slots.length;
        slots.add(
          CircleSlot(
            center: Offset(
              (column - hidden - bleed + 0.5) * slot,
              (row - hidden - bleed + 0.5) * slot,
            ),
            diameter: slot,
            inSight: !isHidden(row) && !isHidden(column),
          ),
        );
      }
    }
    return CircleArrangement(
      slots: slots,
      lanes: [
        for (var row = hidden; row < hidden + count; row += 1)
          [
            for (var column = 0; column < side; column += 1)
              indexes[(row, column)]!,
          ],
        for (var column = hidden; column < hidden + count; column += 1)
          [for (var row = 0; row < side; row += 1) indexes[(row, column)]!],
      ],
      hiddenEnds: hidden,
    );
  }

  /// How many circles are in sight.
  int get inSight => slots.where((slot) => slot.inSight).length;

  /// The largest diameter of a circle in sight, in parts of the tile's side.
  double get largestDiameter => slots
      .where((slot) => slot.inSight)
      .fold(0.0, (largest, slot) => max(largest, slot.diameter));
}

/// A circle on a slot: a cover, or a filler when [cover] is null. [shade]
/// picks the filler's colour, and the colour drawn for a cover that cannot be
/// shown.
@visibleForTesting
@immutable
class RollingCircle {
  final String? cover;
  final int shade;

  const RollingCircle({this.cover, required this.shade});
}

/// What a tile does when its rest ends.
@visibleForTesting
@immutable
sealed class RollingTurn {
  const RollingTurn();
}

/// One fade: the circle on [slot] fades out, takes the cover of the hidden
/// slot [from], and fades in.
@visibleForTesting
class RollingFade extends RollingTurn {
  final int slot;
  final int from;

  const RollingFade({required this.slot, required this.from});
}

/// One roll: the circles of [lane] move [steps] slots on, toward the lane's
/// end when [direction] is 1 and toward its start when it is -1.
@visibleForTesting
class RollingMove extends RollingTurn {
  final int lane;
  final int direction;
  final int steps;

  const RollingMove({
    required this.lane,
    required this.direction,
    required this.steps,
  });
}

/// The circles on the slots of an [arrangement]. No cover is on two circles
/// at once, and the slots in sight take covers before the hidden ones do. A
/// turn leaves as many covers in sight as there were before it, but for a
/// tile that could not move otherwise, which lets one cover go out of sight
/// and brings it back with a later roll.
@visibleForTesting
class RollingCircles {
  final CircleArrangement arrangement;
  final List<String> covers;

  /// How many filler colours there are to pick from.
  final int shades;

  final Random random;

  /// The circle on each slot.
  final List<RollingCircle> _circles;

  final Set<String> _onCircles = {};

  RollingCircles({
    required this.arrangement,
    required List<String> covers,
    required this.shades,
    required this.random,
  }) : covers = covers.toSet().toList(),
       _circles = List.filled(
         arrangement.slots.length,
         const RollingCircle(shade: 0),
       ) {
    // The circles in sight take covers in no order, so that a few covers
    // scatter over the tile, but for a larger circle, which comes first.
    final slots = arrangement.slots;
    final order = [for (var i = 0; i < slots.length; i += 1) i]
      ..shuffle(random);
    for (final first in const [true, false]) {
      final group = [
        for (final index in order)
          if (slots[index].inSight == first) index,
      ];
      mergeSort<int>(
        group,
        compare: (a, b) => slots[b].diameter.compareTo(slots[a].diameter),
      );
      for (final index in group) {
        _put(index, _next());
      }
    }
  }

  /// The circle on the slot at [index].
  RollingCircle at(int index) => _circles[index];

  /// The circles on the slots in sight.
  List<RollingCircle> get circlesInSight => [
    for (final (i, slot) in arrangement.slots.indexed)
      if (slot.inSight) _circles[i],
  ];

  /// How many circles in sight have a cover.
  int get _coversInSight =>
      circlesInSight.where((circle) => circle.cover != null).length;

  /// The covers on circles, in sight or hidden.
  Iterable<String> get coversOnCircles => _onCircles;

  void _put(int index, RollingCircle circle) {
    _circles[index] = circle;
    _onCircles
      ..clear()
      ..addAll(_circles.map((circle) => circle.cover).nonNulls);
  }

  /// A circle with a cover that is on no circle, or else a filler.
  RollingCircle _next() {
    final free = [
      for (final cover in covers)
        if (!_onCircles.contains(cover)) cover,
    ];
    return RollingCircle(
      cover: free.isEmpty ? null : free[random.nextInt(free.length)],
      shade: shades > 0 ? random.nextInt(shades) : 0,
    );
  }

  /// What to do next, picked at random: mostly a roll, and now and then a
  /// fade where the arrangement has a circle for one. A fade also stands in
  /// for a roll when no lane can roll, and when there is no fade either, a
  /// roll that takes one cover out of sight does (see [pickLooseMove]). The
  /// turn is null when there is nothing to do but rest.
  RollingTurn? pickTurn() {
    if (arrangement.fades.isNotEmpty &&
        random.nextDouble() < rollingFadeShare) {
      final fade = pickFade();
      if (fade != null) return fade;
    }
    return pickMove() ?? pickFade() ?? pickLooseMove();
  }

  /// A fade of one of the arrangement's circles for it to a cover that is
  /// out of sight, or null when every cover is in sight.
  RollingFade? pickFade() {
    final fades = arrangement.fades;
    if (fades.isEmpty) return null;
    final hidden = [
      for (final (index, slot) in arrangement.slots.indexed)
        if (!slot.inSight && _circles[index].cover != null) index,
    ];
    if (hidden.isEmpty) return null;
    return RollingFade(
      slot: fades[random.nextInt(fades.length)],
      from: hidden[random.nextInt(hidden.length)],
    );
  }

  /// Changes the cover of [fade], while its circle cannot be seen. The cover
  /// it had goes out of sight, or is let go for one that is on no circle.
  void commitFade(RollingFade fade) {
    final old = _circles[fade.slot];
    _put(fade.slot, _circles[fade.from]);
    _put(fade.from, const RollingCircle(shade: 0));
    final free = _next();
    _put(fade.from, free.cover != null ? free : old);
  }

  /// A roll of one lane, picked at random, or null when no roll keeps the
  /// covers in sight (see [keepsCovers]). A lane is as likely as it has
  /// circles in sight, so most rolls move several circles on the tile, and a
  /// lane with none is never picked. When the roll picked first would take
  /// covers out of sight, another lane, direction or number of steps is
  /// picked among those that would not, a lane again as likely as it has
  /// circles in sight.
  ///
  /// While the tile shows fewer covers than it can, the roll is one that
  /// brings more covers into sight than it takes out, picked in the same
  /// way, when there is one. So a cover that is missing is soon back.
  RollingMove? pickMove() {
    final weights = [
      for (final lane in arrangement.lanes)
        lane.where((index) => arrangement.slots[index].inSight).length,
    ];
    if (_coversInSight < min(covers.length, arrangement.inSight)) {
      final refill = _pickAmong(weights, (move) {
        final moved = coversMoved(move);
        return moved.entering > moved.leaving;
      });
      if (refill != null) return refill;
    }
    var at = random.nextInt(weights.fold(0, (sum, weight) => sum + weight));
    final lane = weights.indexWhere((weight) => (at -= weight) < 0);
    final direction = random.nextBool() ? 1 : -1;
    final steps = 1 + random.nextInt(arrangement.hiddenEnds);
    final first = RollingMove(lane: lane, direction: direction, steps: steps);
    if (keepsCovers(first)) return first;
    return _pickAmong(weights, keepsCovers);
  }

  /// A roll that takes one cover out of sight, for a tile that no roll which
  /// keeps its covers and no fade can move, or null. Only a tile that shows
  /// as many covers as it can is given one, so at most one of its covers is
  /// missing at a time: the cover that left is hidden, and a later roll has
  /// it to bring in. The last cover in sight never leaves. A lane is as
  /// likely as it has circles in sight.
  RollingMove? pickLooseMove() {
    final inSight = _coversInSight;
    if (inSight < min(covers.length, arrangement.inSight)) return null;
    final weights = [
      for (final lane in arrangement.lanes)
        lane.where((index) => arrangement.slots[index].inSight).length,
    ];
    return _pickAmong(weights, (move) {
      final moved = coversMoved(move);
      final lost = moved.leaving - moved.entering;
      return lost <= 1 && inSight - lost >= 1;
    });
  }

  /// One of the rolls that [allowed] takes, of any lane with circles in
  /// sight, direction and number of steps, or null when it takes none. A
  /// lane is as likely as its weight in [weights].
  RollingMove? _pickAmong(
    List<int> weights,
    bool Function(RollingMove move) allowed,
  ) {
    final moves = [
      for (final (lane, weight) in weights.indexed)
        if (weight > 0)
          for (final direction in const [1, -1])
            for (var steps = 1; steps <= arrangement.hiddenEnds; steps += 1)
              RollingMove(lane: lane, direction: direction, steps: steps),
    ].where(allowed).toList();
    if (moves.isEmpty) return null;
    var at = random.nextInt(
      moves.fold(0, (sum, move) => sum + weights[move.lane]),
    );
    return moves.firstWhere((move) => (at -= weights[move.lane]) < 0);
  }

  /// The slots of the lane of [move], in the order its circles move along.
  List<int> laneOf(RollingMove move) {
    final lane = arrangement.lanes[move.lane];
    return move.direction > 0 ? lane : lane.reversed.toList();
  }

  /// Whether the circle at the place [i] of [lane] is in sight once the lane
  /// has rolled [steps] slots on.
  bool _lands(List<int> lane, int i, int steps) =>
      i + steps < lane.length && arrangement.slots[lane[i + steps]].inSight;

  /// The hidden slots of [lane] whose circles come into sight while it rolls
  /// [steps] slots on: first those that are in sight when the roll ends, then
  /// those that only pass through.
  List<int> _entering(List<int> lane, int steps) {
    final slots = arrangement.slots;
    final hidden = [
      for (var i = 0; i < lane.length; i += 1)
        if (!slots[lane[i]].inSight) i,
    ];
    return [
      for (final i in hidden)
        if (_lands(lane, i, steps)) lane[i],
      for (final i in hidden)
        if (!_lands(lane, i, steps) &&
            lane.skip(i + 1).take(steps - 1).any((slot) => slots[slot].inSight))
          lane[i],
    ];
  }

  /// The hidden slots with a cover that [prepare] may give to a filler
  /// coming into sight, which are those not among [entering].
  List<int> _spare(List<int> entering) => [
    for (final (index, slot) in arrangement.slots.indexed)
      if (!slot.inSight &&
          !entering.contains(index) &&
          _circles[index].cover != null)
        index,
  ];

  /// How many covers [move] takes out of sight, and how many it brings into
  /// sight. A cover leaves when its circle is in sight before the roll and
  /// hidden after it. A cover enters when its circle is hidden before the
  /// roll and in sight after it, and that circle counts as a cover when it
  /// has one already or when [prepare] has one to give it: a cover on no
  /// circle, or that of a hidden circle that is not entering.
  ({int leaving, int entering}) coversMoved(RollingMove move) {
    final slots = arrangement.slots;
    final lane = laneOf(move);
    var leaving = 0;
    var entering = 0;
    var fillers = 0;
    for (var i = 0; i < lane.length; i += 1) {
      final isCover = _circles[lane[i]].cover != null;
      final lands = _lands(lane, i, move.steps);
      if (slots[lane[i]].inSight) {
        if (isCover && !lands) leaving += 1;
      } else if (lands && isCover) {
        entering += 1;
      } else if (lands) {
        fillers += 1;
      }
    }
    final spare =
        covers.length -
        _onCircles.length +
        _spare(_entering(lane, move.steps)).length;
    return (leaving: leaving, entering: entering + min(fillers, spare));
  }

  /// Whether [move] leaves as many covers in sight as there are before it,
  /// or more. So the covers of a tile that has no more of them than circles
  /// in sight stay in sight, and only its fillers roll out.
  bool keepsCovers(RollingMove move) {
    final moved = coversMoved(move);
    return moved.entering >= moved.leaving;
  }

  /// Gets the lane ready for [move], before it rolls: a filler about to come
  /// into sight becomes a cover when one is on no circle, or else takes the
  /// cover of a hidden circle that is not entering. The fillers that stay in
  /// sight are served first.
  void prepare(RollingMove move) {
    final entering = _entering(laneOf(move), move.steps);
    for (final index in entering) {
      if (_circles[index].cover != null) continue;
      final free = _next();
      if (free.cover != null) {
        _put(index, free);
        continue;
      }
      final hidden = _spare(entering);
      if (hidden.isEmpty) return;
      final from = hidden[random.nextInt(hidden.length)];
      final filler = _circles[index];
      _put(index, _circles[from]);
      _put(from, filler);
    }
  }

  /// Ends [move]: each circle of the lane is on the slot `steps` on, those
  /// that went past the lane's end are let go, and new ones wait at its
  /// start.
  void commit(RollingMove move) {
    final lane = laneOf(move);
    final before = [for (final index in lane) _circles[index]];
    for (var i = lane.length - 1; i >= 0; i -= 1) {
      // The first slots are fillers until the old circles are off the lane,
      // so that a cover let go can come back.
      _put(
        lane[i],
        i >= move.steps
            ? before[i - move.steps]
            : const RollingCircle(shade: 0),
      );
    }
    for (var i = 0; i < move.steps && i < lane.length; i += 1) {
      _put(lane[i], _next());
    }
  }
}

/// Covers in circles that roll across a tile: now and then the circles of
/// one lane move a slot or two on, those pushed off one edge leave and
/// others enter from the opposite edge. The circles are laid out as
/// [arrangement] says and drawn over [background]. It is still when the
/// platform asks for reduced motion.
class RollingCoverCircles extends StatefulWidget {
  final List<String> paths;

  final CircleArrangementKind arrangement;

  /// The cells along a side of the tile, which sets how many circles it has.
  final int cells;

  /// The side of the tile the covers are decoded for, in logical pixels.
  final int size;

  final Color background;

  /// The colours of the circles that have no cover.
  final List<Color> fillers;

  @visibleForTesting
  final ImageProxy? imageProxy;

  /// The source of the random choices.
  @visibleForTesting
  final Random? random;

  const RollingCoverCircles({
    super.key,
    required this.paths,
    required this.cells,
    required this.size,
    this.arrangement = CircleArrangementKind.square,
    required this.background,
    required this.fillers,
    this.imageProxy,
    this.random,
  });

  @override
  RollingCoverCirclesState createState() => RollingCoverCirclesState();
}

class RollingCoverCirclesState extends State<RollingCoverCircles>
    with SingleTickerProviderStateMixin {
  late final ImageProxy _imageProxy;
  late final Random _random;
  late final AnimationController _controller;
  late RollingCircles _circles;
  final Map<String, ui.Image> _images = {};
  final Set<String> _requested = {};
  final Set<String> _broken = {};
  RollingTurn? _turn;
  Timer? _restTimer;
  Timer? _retryTimer;
  bool? _reduceMotion;
  int? _targetSize;
  int _loadGeneration = 0;

  @visibleForTesting
  RollingCircles get circles => _circles;

  /// The roll under way, or null while the tile rests or fades.
  @visibleForTesting
  RollingMove? get move => switch (_turn) {
    final RollingMove move => move,
    _ => null,
  };

  /// The fade under way, or null while the tile rests or rolls.
  @visibleForTesting
  RollingFade? get fade => switch (_turn) {
    final RollingFade fade => fade,
    _ => null,
  };

  @override
  void initState() {
    super.initState();
    _imageProxy = widget.imageProxy ?? imageMemoryManager.requireProxy();
    _random = widget.random ?? Random();
    _controller = AnimationController(vsync: this);
    _circles = _buildCircles();
  }

  RollingCircles _buildCircles() => RollingCircles(
    arrangement: CircleArrangement.of(widget.arrangement, widget.cells),
    covers: widget.paths,
    shades: widget.fillers.length,
    random: _random,
  );

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _updateTargetSize();
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    if (reduceMotion != _reduceMotion) {
      _reduceMotion = reduceMotion;
      _stop();
      if (!reduceMotion) _rest();
    }
  }

  @override
  void didUpdateWidget(covariant RollingCoverCircles oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!listEquals(oldWidget.paths, widget.paths) ||
        oldWidget.arrangement != widget.arrangement ||
        oldWidget.cells != widget.cells) {
      _stop();
      _circles = _buildCircles();
      _targetSize = null;
      _updateTargetSize();
      if (_reduceMotion == false) _rest();
    } else if (oldWidget.size != widget.size) {
      _updateTargetSize();
    }
  }

  @override
  void dispose() {
    _restTimer?.cancel();
    _retryTimer?.cancel();
    _controller.dispose();
    _images.clear();
    _imageProxy.dispose();
    super.dispose();
  }

  /// Ends the rest and the turn under way, leaving the circles where they
  /// are.
  void _stop() {
    _restTimer?.cancel();
    _restTimer = null;
    _controller.stop();
    _controller.value = 0;
    _turn = null;
  }

  void _rest() {
    _restTimer?.cancel();
    final spread = rollingRestMax - rollingRestMin;
    _restTimer = Timer(rollingRestMin + spread * _random.nextDouble(), _roll);
  }

  void _roll() {
    _restTimer = null;
    if (!mounted || _reduceMotion != false) return;
    final turn = _circles.pickTurn();
    // There is no turn to take: the tile rests once more.
    if (turn == null) {
      _rest();
      return;
    }
    setState(() => _turn = turn);
    bool isOver() => !mounted || !identical(_turn, turn);
    void end() {
      _loadCovers();
      setState(() => _turn = null);
      _controller.value = 0;
      _rest();
    }

    // The futures end only when the turn does, not when it is stopped.
    switch (turn) {
      case RollingMove():
        _circles.prepare(turn);
        _loadCovers();
        _controller.duration = turn.steps == 1
            ? rollingOneSlot
            : rollingTwoSlots;
        _controller.forward(from: 0).then((_) {
          if (isOver()) return;
          _circles.commit(turn);
          end();
        });
      case RollingFade():
        // The circle fades out as the controller runs, and in as it runs
        // back.
        _controller.duration = rollingFade;
        _controller.forward(from: 0).then((_) {
          if (isOver()) return;
          setState(() => _circles.commitFade(turn));
          _controller.reverse(from: 1).then((_) {
            if (isOver()) return;
            end();
          });
        });
    }
  }

  void _updateTargetSize() {
    final diameter = (widget.size * _circles.arrangement.largestDiameter)
        .ceil();
    final targetSize = diameter * MediaQuery.devicePixelRatioOf(context).ceil();
    if (_targetSize == targetSize) return;
    _targetSize = targetSize;
    _loadGeneration += 1;
    // The covers decoded for the old size are not needed any more.
    _imageProxy.dispose();
    _retryTimer?.cancel();
    _retryTimer = null;
    _images.clear();
    _requested.clear();
    _broken.clear();
    _loadCovers();
  }

  /// Asks again for the covers that failed to load, those on circles now and
  /// the others when they come onto one.
  void _retryBroken() {
    _retryTimer = null;
    if (!mounted) return;
    _requested.removeAll(_broken);
    _loadCovers();
  }

  /// Loads the covers on circles that are not loaded yet.
  void _loadCovers() {
    final targetSize = _targetSize;
    if (targetSize == null) return;
    for (final path in _circles.coversOnCircles) {
      if (!_requested.add(path)) continue;
      final cached = _imageProxy.getCachedImage(path, targetSize);
      if (cached != null) {
        _images[path] = cached;
      } else {
        _loadCover(path, targetSize);
      }
    }
  }

  Future<void> _loadCover(String path, int targetSize) async {
    final generation = _loadGeneration;
    ui.Image? image;
    try {
      image = await _imageProxy.requestImage(path, targetSize);
    } catch (_) {
      image = null;
      // The manager keeps a failed load until nothing holds it.
      _imageProxy.release(path, targetSize);
    }
    if (!mounted || generation != _loadGeneration) return;
    setState(() {
      if (image == null) {
        _broken.add(path);
        _retryTimer ??= Timer(rollingCoverRetry, _retryBroken);
      } else {
        _broken.remove(path);
        _images[path] = image;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: CustomPaint(
        painter: RollingCirclesPainter(
          circles: _circles,
          images: _images,
          broken: _broken,
          turn: _turn,
          progress: _controller,
          background: widget.background,
          fillers: widget.fillers,
        ),
        size: Size.infinite,
      ),
    );
  }
}

/// Draws [circles] over [background]. While [turn] is a roll, the circles of
/// its lane are as far along it as [progress] says, and while it is a fade,
/// its circle is as faint.
@visibleForTesting
class RollingCirclesPainter extends CustomPainter {
  final RollingCircles circles;
  final Map<String, ui.Image> images;

  /// The covers that could not be loaded, drawn as fillers.
  final Set<String> broken;

  final RollingTurn? turn;
  final Animation<double> progress;
  final Color background;
  final List<Color> fillers;

  RollingCirclesPainter({
    required this.circles,
    required this.images,
    required this.broken,
    required this.turn,
    required this.progress,
    required this.background,
    required this.fillers,
  }) : super(repaint: progress);

  /// The centre and the diameter [at] a place along [lane], counted in
  /// slots. Past the lane's end a circle keeps the way of the last step.
  (Offset, double) _along(List<int> lane, double at) {
    final slots = circles.arrangement.slots;
    final last = lane.length - 1;
    final from = max(min(at.floor(), last - 1), 0);
    final a = slots[lane[from]];
    final b = slots[lane[min(from + 1, last)]];
    final t = at - from;
    return (
      a.center + (b.center - a.center) * t,
      max(a.diameter + (b.diameter - a.diameter) * t, 0.0),
    );
  }

  /// Where the circle of each slot is drawn on a tile of [size].
  List<Rect> circleRects(Size size) {
    final slots = circles.arrangement.slots;
    final places = [for (final slot in slots) (slot.center, slot.diameter)];
    final move = turn;
    if (move is RollingMove) {
      final lane = circles.laneOf(move);
      final moved = rollingCurve.transform(progress.value) * move.steps;
      for (var i = 0; i < lane.length; i += 1) {
        places[lane[i]] = _along(lane, i + moved);
      }
    }
    return [
      for (final (center, diameter) in places)
        Rect.fromCircle(
          center: center * size.width,
          radius: diameter * size.width / 2,
        ),
    ];
  }

  @override
  void paint(Canvas canvas, Size size) {
    final tile = Offset.zero & size;
    final paint = Paint()..color = background;
    canvas.drawRect(tile, paint);
    final imagePaint = Paint()..filterQuality = FilterQuality.low;
    final fade = turn;

    for (final (index, rect) in circleRects(size).indexed) {
      if (rect.isEmpty || !rect.overlaps(tile)) continue;
      final opacity = fade is RollingFade && fade.slot == index
          ? 1 - progress.value
          : 1.0;
      if (opacity <= 0) continue;
      final circle = circles.at(index);
      final cover = circle.cover;
      final image = cover == null ? null : images[cover];
      final isFiller =
          cover == null ||
          broken.contains(cover) ||
          (image != null && image.width == 1 && image.height == 1);
      if (isFiller) {
        if (fillers.isEmpty) continue;
        final filler = fillers[circle.shade % fillers.length];
        paint.color = filler.withValues(alpha: filler.a * opacity);
        canvas.drawCircle(rect.center, rect.width / 2, paint);
      } else if (image != null) {
        final side = rect.width.ceil();
        canvas.save();
        canvas.clipPath(Path()..addOval(rect));
        canvas.translate(rect.center.dx, rect.center.dy);
        imagePaint.color = Color.fromRGBO(0, 0, 0, opacity);
        drawImageToCanvas(canvas, imagePaint, image, side, side);
        canvas.restore();
      }
    }
  }

  @override
  bool shouldRepaint(covariant RollingCirclesPainter oldDelegate) => true;
}
