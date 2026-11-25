import 'dart:async';
import 'dart:math';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/tile/rolling_cover_circles.dart';
import 'package:rune/widgets/tile/utils/image_memory_manager.dart';
import 'package:rune/widgets/tile/utils/image_proxy.dart';

class _ImageProxy extends ImageProxy {
  _ImageProxy() : super(manager: ImageMemoryManager());
  final requests = <String, Completer<ui.Image>>{};
  final asked = <String>[];
  final released = <(String, int)>[];
  var disposals = 0;
  final sizes = <int>{};
  @override
  ui.Image? getCachedImage(String path, int size) => null;
  @override
  Future<ui.Image> requestImage(String path, int size) {
    sizes.add(size);
    asked.add(path);
    return (requests[path] = Completer<ui.Image>()).future;
  }

  @override
  void release(String path, int size) => released.add((path, size));

  @override
  void dispose() => disposals += 1;
}

Future<ui.Image> _image() async {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  canvas.drawRect(const Rect.fromLTWH(0, 0, 2, 2), Paint()..color = Colors.red);
  return recorder.endRecording().toImage(2, 2);
}

List<String> _covers(int count) => [for (var i = 0; i < count; i += 1) 'c$i'];

RollingCircles _circles(int covers, int count, {int seed = 1}) =>
    RollingCircles(
      arrangement: CircleArrangement.square(count),
      covers: _covers(covers),
      shades: 4,
      random: Random(seed),
    );

/// The covers on every circle, one entry for each circle with a cover.
List<String> _onCircles(RollingCircles circles) => [
  for (var i = 0; i < circles.arrangement.slots.length; i += 1)
    if (circles.at(i).cover != null) circles.at(i).cover!,
];

List<String> _inSight(RollingCircles circles) => [
  for (final circle in circles.circlesInSight)
    if (circle.cover != null) circle.cover!,
];

void _expectNoCoverTwice(RollingCircles circles, String reason) {
  final covers = _onCircles(circles);
  expect(covers.toSet(), hasLength(covers.length), reason: reason);
  expect(circles.coversOnCircles.toSet(), covers.toSet(), reason: reason);
}

/// Every roll of [circles], of a lane with circles in sight.
List<RollingMove> _allRolls(RollingCircles circles) => [
  for (final (lane, slots) in circles.arrangement.lanes.indexed)
    if (slots.any((i) => circles.arrangement.slots[i].inSight))
      for (final direction in [1, -1])
        for (final steps in [1, 2])
          RollingMove(lane: lane, direction: direction, steps: steps),
];

const _background = Color(0xFF102030);
const _fillers = [
  Color(0xFF000001),
  Color(0xFF000002),
  Color(0xFF000003),
  Color(0xFF000004),
];

void main() {
  group('the square arrangement', () {
    for (final count in [2, 3]) {
      test('of $count has touching circles that reach past the edges', () {
        final arrangement = CircleArrangement.square(count);
        final slot = 1 / (count - 0.4);
        final inSight = arrangement.slots.where((s) => s.inSight).toList();
        expect(inSight, hasLength(count * count));
        for (final s in arrangement.slots) {
          expect(s.diameter, closeTo(slot, 1e-9));
        }
        // The first circle starts a fifth of itself before the edge, and the
        // last one ends as far past the opposite edge.
        expect(inSight.first.center.dx - slot / 2, closeTo(-0.2 * slot, 1e-9));
        expect(inSight.first.center.dy - slot / 2, closeTo(-0.2 * slot, 1e-9));
        expect(
          inSight.last.center.dx + slot / 2,
          closeTo(1 + 0.2 * slot, 1e-9),
        );
        // Every hidden slot lies off the tile.
        for (final s in arrangement.slots.where((s) => !s.inSight)) {
          final rect = Rect.fromCircle(center: s.center, radius: slot / 2);
          expect(rect.overlaps(const Rect.fromLTWH(0, 0, 1, 1)), isFalse);
        }
        expect(arrangement.largestDiameter, closeTo(slot, 1e-9));
      });

      test('of $count has rows and columns as lanes, with two hidden slots '
          'at each end', () {
        final arrangement = CircleArrangement.square(count);
        expect(arrangement.hiddenEnds, 2);
        expect(arrangement.lanes, hasLength(count * 2));
        for (final lane in arrangement.lanes) {
          expect(lane, hasLength(count + 4));
          expect(
            [for (final i in lane) arrangement.slots[i].inSight],
            [
              false,
              false,
              for (var i = 0; i < count; i += 1) true,
              false,
              false,
            ],
          );
          // Neighbours along a lane touch.
          for (var i = 1; i < lane.length; i += 1) {
            final a = arrangement.slots[lane[i - 1]];
            final b = arrangement.slots[lane[i]];
            expect((b.center - a.center).distance, closeTo(a.diameter, 1e-9));
          }
        }
        // Every slot is on a lane.
        expect(
          arrangement.lanes.expand((lane) => lane).toSet(),
          hasLength(arrangement.slots.length),
        );
      });
    }
  });

  /// Every arrangement a tile can have, by its name.
  final arrangements = <String, CircleArrangement>{
    for (final kind in CircleArrangementKind.values)
      for (final cells in [1, 2])
        if (kind != CircleArrangementKind.bigAndSmall || cells == 2)
          '${kind.name} on $cells': CircleArrangement.of(kind, cells),
  };

  /// Whether the circle of [slot] reaches into the tile.
  bool reachesIn(CircleSlot slot) {
    final dx = max(max(-slot.center.dx, slot.center.dx - 1), 0.0);
    final dy = max(max(-slot.center.dy, slot.center.dy - 1), 0.0);
    return sqrt(dx * dx + dy * dy) < slot.diameter / 2 - 1e-9;
  }

  group('every arrangement', () {
    arrangements.forEach((name, arrangement) {
      final slots = arrangement.slots;

      test('$name has touching circles along its lanes, with hidden slots '
          'for a roll of two at each end', () {
        expect(arrangement.hiddenEnds, 2);
        expect(arrangement.lanes, isNotEmpty);
        for (final lane in arrangement.lanes) {
          final inSight = [for (final i in lane) slots[i].inSight];
          expect(inSight.length, greaterThanOrEqualTo(5));
          expect(inSight.take(2), [false, false]);
          expect(inSight.skip(inSight.length - 2), [false, false]);
          expect(
            inSight.sublist(2, inSight.length - 2),
            everyElement(isTrue),
            reason: 'a lane is in sight between its hidden ends',
          );
          expect(lane.toSet(), hasLength(lane.length));
          for (var i = 1; i < lane.length; i += 1) {
            final a = slots[lane[i - 1]];
            final b = slots[lane[i]];
            expect(
              (b.center - a.center).distance,
              closeTo((a.diameter + b.diameter) / 2, 1e-9),
            );
          }
        }
      });

      test('$name has its slots in sight on the tile and the others off it, '
          'none over another', () {
        for (final slot in slots) {
          expect(slot.inSight, reachesIn(slot));
        }
        for (var i = 0; i < slots.length; i += 1) {
          for (var j = i + 1; j < slots.length; j += 1) {
            expect(
              (slots[i].center - slots[j].center).distance,
              greaterThan((slots[i].diameter + slots[j].diameter) / 2 - 1e-9),
            );
          }
        }
        // Every circle either rolls or fades.
        final onLanes = arrangement.lanes.expand((lane) => lane).toSet();
        expect({...onLanes, ...arrangement.fades}, hasLength(slots.length));
        expect(onLanes.intersection(arrangement.fades.toSet()), isEmpty);
        for (final index in arrangement.fades) {
          expect(slots[index].inSight, isTrue);
        }
      });

      test('$name rolls a lane as often as it has circles in sight', () {
        final circles = RollingCircles(
          arrangement: arrangement,
          covers: _covers(60),
          shades: 4,
          random: Random(11),
        );
        final inSight = [
          for (final lane in arrangement.lanes)
            lane.where((i) => slots[i].inSight).length,
        ];
        final total = inSight.reduce((a, b) => a + b);
        const picks = 40000;
        final picked = List.filled(arrangement.lanes.length, 0);
        for (var i = 0; i < picks; i += 1) {
          picked[circles.pickMove()!.lane] += 1;
        }
        for (final (lane, count) in picked.indexed) {
          expect(
            count / picks,
            closeTo(inSight[lane] / total, 0.01),
            reason: 'lane $lane with ${inSight[lane]} in sight',
          );
        }
      });

      test('$name never rolls a lane with no circle in sight', () {
        // The same slots with one more lane, of hidden slots only.
        final hidden = [
          for (final (i, slot) in slots.indexed)
            if (!slot.inSight) i,
        ];
        final circles = RollingCircles(
          arrangement: CircleArrangement(
            slots: slots,
            lanes: [hidden.take(4).toList(), ...arrangement.lanes],
            hiddenEnds: arrangement.hiddenEnds,
          ),
          covers: _covers(60),
          shades: 4,
          random: Random(12),
        );
        for (var i = 0; i < 5000; i += 1) {
          expect(circles.pickMove()!.lane, isNot(0));
        }
      });

      for (final covers in [1, 3, 5, 20]) {
        test('$name keeps its covers in sight through every turn but one at '
            'a time, with $covers covers', () {
          final circles = RollingCircles(
            arrangement: arrangement,
            covers: _covers(covers),
            shades: 4,
            random: Random(covers * 7),
          );
          final most = min(covers, arrangement.inSight);
          expect(_inSight(circles), hasLength(most));
          final seen = _inSight(circles).toSet();
          var strict = 0;
          var loose = 0;
          for (var turn = 0; turn < 2000; turn += 1) {
            final before = _inSight(circles).length;
            final canKeep = _allRolls(circles).any(circles.keepsCovers);
            switch (circles.pickTurn()) {
              case final RollingMove move:
                final moved = circles.coversMoved(move);
                if (circles.keepsCovers(move)) {
                  strict += 1;
                } else {
                  // A cover only leaves when no roll keeps them all, and
                  // none is missing yet.
                  loose += 1;
                  expect(canKeep, isFalse, reason: 'turn $turn');
                  expect(before, most, reason: 'turn $turn');
                  expect(moved.leaving - moved.entering, 1);
                }
                circles.prepare(move);
                circles.commit(move);
                // The roll moved the covers it was counted to move.
                expect(
                  _inSight(circles).length - before,
                  moved.entering - moved.leaving,
                  reason: 'turn $turn',
                );
              case final RollingFade fade:
                circles.commitFade(fade);
                expect(_inSight(circles), hasLength(before));
              case null:
                // Only the last cover in sight keeps a tile still.
                expect(before, 1, reason: 'turn $turn');
                expect(canKeep, isFalse, reason: 'turn $turn');
            }
            expect(
              _inSight(circles).length,
              greaterThanOrEqualTo(max(most - 1, 1)),
              reason: 'turn $turn',
            );
            _expectNoCoverTwice(circles, 'turn $turn');
            seen.addAll(_inSight(circles));
          }
          if (covers > arrangement.inSight) {
            // There is always a cover to come in: every turn keeps them
            // all, and the tile shows all of its covers in time.
            expect(loose, 0);
            expect(strict, greaterThan(1000));
            expect(seen, _covers(covers).toSet());
          }
        });
      }

      for (final covers in [1, 2, 5, 8, 12, 16, 60]) {
        test('$name never shows one of $covers covers twice', () {
          final circles = RollingCircles(
            arrangement: arrangement,
            covers: _covers(covers),
            shades: 4,
            random: Random(covers),
          );
          _expectNoCoverTwice(circles, 'at first');
          final most = min(covers, circles.circlesInSight.length);
          expect(_inSight(circles), hasLength(most));
          var fades = 0;
          for (var turn = 0; turn < 300; turn += 1) {
            switch (circles.pickTurn()) {
              case final RollingMove move:
                circles.prepare(move);
                _expectNoCoverTwice(circles, 'turn $turn, rolling');
                circles.commit(move);
              case final RollingFade fade:
                fades += 1;
                final inSight = _inSight(circles);
                circles.commitFade(fade);
                expect(
                  inSight,
                  isNot(contains(circles.at(fade.slot).cover)),
                  reason: 'turn $turn fades to a cover that was out of sight',
                );
                expect(_inSight(circles), hasLength(inSight.length));
              case null:
                expect(_inSight(circles), hasLength(1));
            }
            _expectNoCoverTwice(circles, 'turn $turn, at rest');
            expect(
              _inSight(circles).length,
              greaterThanOrEqualTo(max(most - 1, 1)),
              reason: 'turn $turn',
            );
          }
          if (arrangement.fades.isEmpty) expect(fades, 0);
        });
      }
    });
  });

  group('a tile with as many covers as circles in sight', () {
    for (final (name, kind, cells, covers) in const [
      ('the square on one cell', CircleArrangementKind.square, 1, 4),
      ('the big and small', CircleArrangementKind.bigAndSmall, 2, 6),
      ('the square on two cells', CircleArrangementKind.square, 2, 9),
    ]) {
      test('keeps rolling, $name with $covers covers', () {
        for (var seed = 0; seed < 20; seed += 1) {
          final circles = RollingCircles(
            arrangement: CircleArrangement.of(kind, cells),
            covers: _covers(covers),
            shades: 4,
            random: Random(seed),
          );
          expect(circles.arrangement.inSight, covers);
          var rolls = 0;
          var full = 0;
          final places = <String>{};
          for (var turn = 0; turn < 1000; turn += 1) {
            final before = _inSight(circles).length;
            // Whether a roll can bring the missing cover back.
            final canRefill = _allRolls(circles).any((move) {
              final moved = circles.coversMoved(move);
              return moved.entering > moved.leaving;
            });
            final picked = circles.pickTurn();
            expect(picked, isNotNull, reason: '$seed, turn $turn');
            switch (picked!) {
              case final RollingMove move:
                rolls += 1;
                circles.prepare(move);
                circles.commit(move);
                if (before < covers && canRefill) {
                  expect(
                    _inSight(circles),
                    hasLength(covers),
                    reason:
                        '$seed, turn $turn: the roll after one that let '
                        'a cover go brings it back',
                  );
                }
              case final RollingFade fade:
                circles.commitFade(fade);
            }
            final inSight = _inSight(circles).length;
            expect(inSight, inInclusiveRange(covers - 1, covers));
            if (inSight == covers) full += 1;
            _expectNoCoverTwice(circles, '$seed, turn $turn');
            places.add(
              circles.circlesInSight.map((c) => c.cover ?? '-').join(','),
            );
          }
          expect(rolls, greaterThan(500), reason: '$seed');
          // The missing cover is soon back, and the covers change places.
          // A fade, which three turns of ten are where there is a circle
          // for one, changes a cover while another is missing, and leaves
          // the tile as it is.
          final fades = circles.arrangement.fades.isNotEmpty;
          expect(
            full,
            greaterThanOrEqualTo(fades ? 330 : 400),
            reason: '$seed',
          );
          expect(places.length, greaterThan(50), reason: '$seed');
        }
      });
    }
  });

  group('the hex arrangement', () {
    test('sets each row off by half a circle', () {
      for (final (cells, across) in [(1, 2.2), (2, 3.2)]) {
        final arrangement = CircleArrangement.of(
          CircleArrangementKind.hex,
          cells,
        );
        final d = 1 / across;
        final rows = <double, List<double>>{};
        for (final slot in arrangement.slots) {
          expect(slot.diameter, closeTo(d, 1e-9));
          final row = ((slot.center.dy - d * 0.45) / (d * sqrt(3) / 2));
          expect(row, closeTo(row.round(), 1e-6));
          rows.putIfAbsent(row.roundToDouble(), () => []).add(slot.center.dx);
        }
        // The row through the first point starts three tenths of a circle
        // in, and the next one half a circle further.
        double phase(double x) => ((x / d - 0.3) % 1 + 1) % 1;
        expect(
          phase(rows[0]!.first),
          anyOf(closeTo(0, 1e-6), closeTo(1, 1e-6)),
        );
        expect(phase(rows[1]!.first), closeTo(0.5, 1e-6));
        // Rows and both diagonals are lanes.
        final ways = {
          for (final lane in arrangement.lanes)
            ((arrangement.slots[lane[1]].center -
                            arrangement.slots[lane[0]].center)
                        .direction *
                    180 /
                    pi)
                .round(),
        };
        expect(ways, {0, 60, 120});
      }
    });
  });

  group('the diamond arrangement', () {
    test('is the square lattice turned by an eighth about the middle', () {
      for (final (cells, across) in [(1, 1.6), (2, 2.4)]) {
        final arrangement = CircleArrangement.of(
          CircleArrangementKind.diamond,
          cells,
        );
        final d = 1 / across;
        expect(
          arrangement.slots.where(
            (slot) => (slot.center - const Offset(0.5, 0.5)).distance < 1e-9,
          ),
          hasLength(1),
        );
        for (final slot in arrangement.slots) {
          expect(slot.diameter, closeTo(d, 1e-9));
        }
        final ways = {
          for (final lane in arrangement.lanes)
            ((arrangement.slots[lane[1]].center -
                            arrangement.slots[lane[0]].center)
                        .direction *
                    180 /
                    pi)
                .round(),
        };
        expect(ways, {45, 135});
      }
    });
  });

  group('the big and small arrangement', () {
    final arrangement = CircleArrangement.of(
      CircleArrangementKind.bigAndSmall,
      2,
    );
    const small = 1 / 2.6;

    test('has one circle of twice the size at the top left, on no lane', () {
      final big = arrangement.slots[arrangement.fades.single];
      expect(big.diameter, closeTo(small * 2, 1e-9));
      expect(big.center.dx, closeTo(small * 0.8, 1e-9));
      expect(big.center.dy, closeTo(small * 0.8, 1e-9));
      expect(arrangement.largestDiameter, closeTo(small * 2, 1e-9));
      expect(
        arrangement.lanes.expand((lane) => lane),
        isNot(contains(arrangement.fades.single)),
      );
    });

    test('has a column and a row of small circles that share the corner', () {
      expect(arrangement.lanes, hasLength(2));
      final [column, row] = arrangement.lanes;
      List<CircleSlot> inSight(List<int> lane) => [
        for (final i in lane)
          if (arrangement.slots[i].inSight) arrangement.slots[i],
      ];
      expect(inSight(column), hasLength(3));
      expect(inSight(row), hasLength(3));
      for (final slot in [...inSight(column), ...inSight(row)]) {
        expect(slot.diameter, closeTo(small, 1e-9));
      }
      expect(inSight(column).map((s) => s.center.dx).toSet(), hasLength(1));
      expect(inSight(row).map((s) => s.center.dy).toSet(), hasLength(1));
      expect(column.toSet().intersection(row.toSet()), hasLength(1));
      expect(inSight(row).last, same(inSight(column).last));
      // Five small circles and the large one.
      expect(arrangement.slots.where((s) => s.inSight), hasLength(6));
    });

    test('is the square one on a tile of one cell', () {
      final one = CircleArrangement.of(CircleArrangementKind.bigAndSmall, 1);
      expect(one.fades, isEmpty);
      expect(one.slots.where((s) => s.inSight), hasLength(4));
    });

    test('gives the large circle a cover first', () {
      for (var seed = 0; seed < 20; seed += 1) {
        final circles = RollingCircles(
          arrangement: arrangement,
          covers: const ['a'],
          shades: 4,
          random: Random(seed),
        );
        expect(circles.at(arrangement.fades.single).cover, 'a');
      }
    });

    test('fades the large circle to a cover that is out of sight', () {
      final circles = RollingCircles(
        arrangement: arrangement,
        covers: _covers(9),
        shades: 4,
        random: Random(3),
      );
      for (var i = 0; i < 50; i += 1) {
        final fade = circles.pickFade()!;
        expect(fade.slot, arrangement.fades.single);
        expect(arrangement.slots[fade.from].inSight, isFalse);
        final before = _inSight(circles);
        final old = circles.at(fade.slot).cover;
        final next = circles.at(fade.from).cover;
        expect(next, isNotNull);
        expect(before, isNot(contains(next)));
        circles.commitFade(fade);
        expect(circles.at(fade.slot).cover, next);
        // The cover it had is out of sight now.
        expect(_inSight(circles), isNot(contains(old)));
        _expectNoCoverTwice(circles, 'fade $i');
      }
    });

    test('lets the cover go for one on no circle when there is one', () {
      final circles = RollingCircles(
        arrangement: arrangement,
        covers: _covers(60),
        shades: 4,
        random: Random(3),
      );
      final fade = circles.pickFade()!;
      final old = circles.at(fade.slot).cover;
      circles.commitFade(fade);
      expect(circles.coversOnCircles, isNot(contains(old)));
      expect(circles.at(fade.from).cover, isNotNull);
      _expectNoCoverTwice(circles, 'after the fade');
    });

    test('lets one cover go when every circle in sight has a cover and no '
        'other cover is left', () {
      final circles = RollingCircles(
        arrangement: arrangement,
        covers: _covers(6),
        shades: 4,
        random: Random(3),
      );
      expect(circles.pickMove(), isNull);
      expect(circles.pickFade(), isNull);
      final move = circles.pickTurn()! as RollingMove;
      circles.prepare(move);
      circles.commit(move);
      expect(_inSight(circles), hasLength(5));
      // The cover that left is hidden, so no other cover leaves, and a roll
      // or a fade can bring it back.
      expect(circles.pickLooseMove(), isNull);
      expect(_onCircles(circles), hasLength(6));
      expect(circles.pickMove(), isNotNull);
    });

    test('rolls one slot when a roll of two would push covers out', () {
      // One cover more than the circles in sight, on a hidden slot of the
      // column, whose start it is two slots from. A roll of two slots along
      // either lane pushes two covers out and brings at most one in, but a
      // roll of one slot keeps the covers in sight, so it is the one picked.
      for (var seed = 0; seed < 20; seed += 1) {
        final circles = RollingCircles(
          arrangement: arrangement,
          covers: _covers(7),
          shades: 4,
          random: Random(seed),
        );
        for (final (lane, _) in arrangement.lanes.indexed) {
          for (final direction in [1, -1]) {
            final move = RollingMove(
              lane: lane,
              direction: direction,
              steps: 2,
            );
            expect(circles.coversMoved(move).leaving, 2);
            expect(circles.keepsCovers(move), isFalse);
          }
        }
        expect(circles.pickMove()?.steps, 1, reason: 'seed $seed');
      }
    });

    test('fades in about three turns of ten', () {
      final circles = RollingCircles(
        arrangement: arrangement,
        covers: _covers(60),
        shades: 4,
        random: Random(5),
      );
      var fades = 0;
      for (var i = 0; i < 1000; i += 1) {
        if (circles.pickTurn() is RollingFade) fades += 1;
      }
      expect(fades, inInclusiveRange(250, 350));
    });
  });

  group('the circles', () {
    test('in sight take covers before the hidden ones', () {
      // Fewer covers than circles in sight: all are in sight.
      var circles = _circles(5, 3);
      expect(_inSight(circles).toSet(), _covers(5).toSet());
      expect(_onCircles(circles), hasLength(5));
      // More: the circles in sight are all covers and the rest are hidden.
      circles = _circles(12, 3);
      expect(_inSight(circles), hasLength(9));
      expect(_onCircles(circles).toSet(), _covers(12).toSet());
    });

    test('without a cover are fillers in one of the shades', () {
      final circles = _circles(1, 2);
      final fillers = circles.circlesInSight.where((c) => c.cover == null);
      expect(fillers, hasLength(3));
      final all = [
        for (var i = 0; i < circles.arrangement.slots.length; i += 1)
          circles.at(i),
      ];
      expect(all.map((c) => c.shade), everyElement(inInclusiveRange(0, 3)));
      expect(all.map((c) => c.shade).toSet().length, greaterThan(1));
    });

    test('count a cover given twice once', () {
      final circles = RollingCircles(
        arrangement: CircleArrangement.square(2),
        covers: const ['a', 'b', 'a', 'a'],
        shades: 4,
        random: Random(1),
      );
      expect(_onCircles(circles).toSet(), {'a', 'b'});
      expect(_onCircles(circles), hasLength(2));
    });

    for (final count in [2, 3]) {
      for (final covers in [1, 2, 5, 8, 9, 12, 16, 60]) {
        test('never show a cover twice, $covers covers on $count a side', () {
          final circles = _circles(covers, count, seed: covers * 10 + count);
          _expectNoCoverTwice(circles, 'at first');
          final most = min(covers, count * count);
          for (var roll = 0; roll < 300; roll += 1) {
            final move = circles.pickMove();
            if (move == null) continue;
            expect(move.steps, inInclusiveRange(1, 2));
            expect(move.direction.abs(), 1);
            circles.prepare(move);
            // While the lane rolls, the circles of every slot may be seen.
            _expectNoCoverTwice(circles, 'roll $roll, rolling');
            circles.commit(move);
            _expectNoCoverTwice(circles, 'roll $roll, at rest');
            // A roll brings as many covers into sight as it pushes out.
            expect(_inSight(circles), hasLength(most), reason: 'roll $roll');
          }
        });
      }
    }

    test('of a lane move on by the steps of a roll', () {
      final circles = _circles(60, 3);
      for (final direction in [1, -1]) {
        for (final steps in [1, 2]) {
          final move = RollingMove(lane: 4, direction: direction, steps: steps);
          final lane = circles.laneOf(move);
          final others = {
            for (var i = 0; i < circles.arrangement.slots.length; i += 1)
              if (!lane.contains(i)) i: circles.at(i),
          };
          circles.prepare(move);
          final before = [for (final i in lane) circles.at(i)];
          circles.commit(move);
          final after = [for (final i in lane) circles.at(i)];
          expect(after.sublist(steps), before.sublist(0, lane.length - steps));
          // New covers wait at the start of the lane.
          for (final circle in after.take(steps)) {
            expect(circle.cover, isNotNull);
            expect(before.map((c) => c.cover), isNot(contains(circle.cover)));
          }
          others.forEach((i, circle) => expect(circles.at(i), same(circle)));
        }
      }
    });

    test('entering are covers while one is out of sight', () {
      // Nine covers, all in sight: a filler enters.
      final circles = _circles(9, 3);
      const move = RollingMove(lane: 0, direction: 1, steps: 1);
      final lane = circles.laneOf(move);
      circles.prepare(move);
      expect(circles.at(lane[1]).cover, isNull);
      circles.commit(move);
      expect(_inSight(circles), hasLength(8));
      // The cover pushed out is hidden, and enters with the next roll of
      // another lane.
      const next = RollingMove(lane: 4, direction: -1, steps: 1);
      final nextLane = circles.laneOf(next);
      circles.prepare(next);
      expect(circles.at(nextLane[1]).cover, isNotNull);
      _expectNoCoverTwice(circles, 'second roll');
    });
  });

  group('the widget', () {
    Widget app(
      _ImageProxy proxy, {
      List<String> paths = const ['a', 'b'],
      int cells = 2,
      CircleArrangementKind arrangement = CircleArrangementKind.square,
      bool disableAnimations = false,
      int seed = 1,
      int size = 252,
    }) => FluentApp(
      home: MediaQuery(
        data: MediaQueryData(
          size: const Size(800, 600),
          devicePixelRatio: 2,
          disableAnimations: disableAnimations,
        ),
        child: Center(
          child: SizedBox(
            width: 252,
            height: 252,
            child: RollingCoverCircles(
              paths: paths,
              cells: cells,
              arrangement: arrangement,
              size: size,
              background: _background,
              fillers: _fillers,
              imageProxy: proxy,
              random: Random(seed),
            ),
          ),
        ),
      ),
    );

    RollingCoverCirclesState state(WidgetTester tester) =>
        tester.state(find.byType(RollingCoverCircles));

    Finder paint() => find.descendant(
      of: find.byType(RollingCoverCircles),
      matching: find.byType(CustomPaint),
    );

    RollingCirclesPainter painter(WidgetTester tester) =>
        tester.widget<CustomPaint>(paint()).painter! as RollingCirclesPainter;

    /// Waits out a rest, so that a roll is under way.
    Future<void> startRoll(WidgetTester tester) async {
      await tester.pump(rollingRestMax);
      expect(state(tester).move, isNotNull);
    }

    /// Lets the roll under way end.
    Future<void> endRoll(WidgetTester tester) async {
      await tester.pump();
      // A roll ends with the first frame after its time.
      await tester.pump(rollingTwoSlots + const Duration(milliseconds: 1));
      await tester.pump();
      expect(state(tester).move, isNull);
    }

    testWidgets('draws fillers in the filler colours over the background', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      expect(painter(tester).background, _background);
      expect(painter(tester).fillers, _fillers);
      // Two covers, not loaded yet, and seven fillers in sight.
      expect(paint(), paints..rect(color: _background));
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 7));
      expect(paint(), paintsExactlyCountTimes(#drawImageRect, 0));
      final first = state(
        tester,
      ).circles.circlesInSight.firstWhere((c) => c.cover == null);
      expect(
        paint(),
        paints
          ..rect(color: _background)
          ..circle(color: _fillers[first.shade]),
      );
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('draws each cover in a circle, decoded for the circle', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      expect(proxy.requests.keys.toSet(), {'a', 'b'});
      // A circle of a tile of 252 with three a side, at two pixels a point.
      expect(proxy.sizes, {(252 / 2.6).ceil() * 2});
      final image = await _image();
      proxy.requests['a']!.complete(image);
      proxy.requests['b']!.complete(image);
      await tester.pump();
      expect(paint(), paintsExactlyCountTimes(#drawImageRect, 2));
      expect(paint(), paintsExactlyCountTimes(#clipPath, 2));
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 7));
      await tester.pumpWidget(const SizedBox());
      image.dispose();
    });

    testWidgets('draws a cover that cannot be loaded as a filler', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      proxy.requests['a']!.completeError(StateError('no file'));
      await tester.pump();
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 8));
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('asks again for a cover that could not be loaded, after a '
        'while', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      proxy.requests['a']!.completeError(StateError('no file'));
      await tester.pump();
      // The failed load is let go of, or the manager would answer it again.
      expect(proxy.released, [('a', (252 / 2.6).ceil() * 2)]);
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 8));

      await tester.pump(rollingCoverRetry - const Duration(seconds: 1));
      expect(proxy.asked.where((path) => path == 'a'), hasLength(1));
      await tester.pump(const Duration(seconds: 1));
      expect(proxy.asked.where((path) => path == 'a'), hasLength(2));
      // Still a filler while it loads, and a cover when it has loaded.
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 8));
      final image = await _image();
      proxy.requests['a']!.complete(image);
      await tester.pump();
      expect(paint(), paintsExactlyCountTimes(#drawImageRect, 1));
      expect(paint(), paintsExactlyCountTimes(#drawCircle, 7));
      await tester.pumpWidget(const SizedBox());
      image.dispose();
    });

    testWidgets('keeps asking while a cover cannot be loaded, not at once', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      for (var round = 1; round <= 3; round += 1) {
        proxy.requests['a']!.completeError(StateError('no file'));
        await tester.pump();
        await tester.pump(const Duration(seconds: 10));
        expect(proxy.asked.where((path) => path == 'a'), hasLength(round));
        await tester.pump(rollingCoverRetry - const Duration(seconds: 10));
        expect(proxy.asked.where((path) => path == 'a'), hasLength(round + 1));
      }
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('lets go of the covers decoded for a size it has outgrown', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      final before = proxy.disposals;
      await tester.pumpWidget(app(proxy, size: 400));
      expect(proxy.disposals, before + 1);
      expect(proxy.sizes, {(252 / 2.6).ceil() * 2, (400 / 2.6).ceil() * 2});
      // An unchanged size keeps the covers.
      await tester.pumpWidget(app(proxy, size: 400));
      expect(proxy.disposals, before + 1);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('rests, then rolls one lane and rests again', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy, paths: _covers(12)));
      final size = tester.getSize(paint());
      final atRest = painter(tester).circleRects(size);
      // Neighbours touch on the tile as well.
      expect(atRest.first.width, closeTo(252 / 2.6, 1e-6));

      await tester.pump(rollingRestMin - const Duration(milliseconds: 1));
      expect(state(tester).move, isNull);
      expect(tester.binding.transientCallbackCount, 0);

      await startRoll(tester);
      final move = state(tester).move!;
      final circles = state(tester).circles;
      final lane = circles.laneOf(move);
      final before = [for (final i in lane) circles.at(i)];
      expect(tester.binding.transientCallbackCount, 1);

      // Just before its end, the lane is nearly its steps on, and no other
      // circle has moved.
      await tester.pump();
      final length = move.steps == 1 ? rollingOneSlot : rollingTwoSlots;
      await tester.pump(length - const Duration(milliseconds: 1));
      expect(state(tester).move, same(move));
      final rolling = painter(tester).circleRects(size);
      for (var i = 0; i < atRest.length; i += 1) {
        final at = lane.indexOf(i);
        if (at < 0) {
          expect(rolling[i], atRest[i]);
        } else if (at + move.steps < lane.length) {
          final target = atRest[lane[at + move.steps]];
          expect((rolling[i].center - target.center).distance, lessThan(0.5));
        }
      }

      await tester.pump(const Duration(milliseconds: 2));
      await tester.pump();
      expect(state(tester).move, isNull);
      expect(tester.binding.transientCallbackCount, 0);
      expect(painter(tester).circleRects(size), atRest);
      final after = [for (final i in lane) circles.at(i)];
      expect(
        after.sublist(move.steps),
        before.sublist(0, lane.length - move.steps),
      );
      // The covers that came onto circles are asked for.
      expect(proxy.requests.keys, containsAll(circles.coversOnCircles));

      // The next roll follows a rest.
      await tester.pump(rollingRestMin - const Duration(milliseconds: 10));
      expect(state(tester).move, isNull);
      await startRoll(tester);
      await endRoll(tester);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('keeps rolling with as many covers as circles in sight', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy, paths: _covers(9)));
      final circles = state(tester).circles;
      expect(_inSight(circles), hasLength(9));
      for (var turn = 0; turn < 12; turn += 1) {
        await startRoll(tester);
        await endRoll(tester);
        expect(_inSight(circles).length, inInclusiveRange(8, 9));
        _expectNoCoverTwice(circles, 'turn $turn');
      }
      await tester.pumpWidget(const SizedBox());
    });

    for (final covers in [2, 12, 20]) {
      testWidgets('never shows one of $covers covers twice while it rolls', (
        tester,
      ) async {
        final proxy = _ImageProxy();
        await tester.pumpWidget(app(proxy, paths: _covers(covers), seed: 7));
        final circles = state(tester).circles;
        _expectNoCoverTwice(circles, 'at first');
        final first = _inSight(circles).length;
        expect(first, min(covers, 9));
        for (var roll = 0; roll < 25; roll += 1) {
          await startRoll(tester);
          _expectNoCoverTwice(circles, 'roll $roll, rolling');
          await endRoll(tester);
          _expectNoCoverTwice(circles, 'roll $roll, at rest');
          expect(_inSight(circles), hasLength(first));
        }
        await tester.pumpWidget(const SizedBox());
      });
    }

    testWidgets('tiles roll at their own times', (tester) async {
      final rests = <int>{};
      for (final seed in [1, 2, 3, 4]) {
        final proxy = _ImageProxy();
        await tester.pumpWidget(app(proxy, seed: seed));
        var waited = 0;
        // The longest rest, and a moment more, ends in a roll.
        while (state(tester).move == null && waited <= 11100) {
          await tester.pump(const Duration(milliseconds: 100));
          waited += 100;
        }
        expect(state(tester).move, isNotNull, reason: 'seed $seed');
        expect(waited, inInclusiveRange(6000, 11100));
        rests.add(waited);
        await tester.pumpWidget(const SizedBox());
      }
      expect(rests.length, greaterThan(1));
    });

    testWidgets('is still when the platform asks for reduced motion', (
      tester,
    ) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(
        app(proxy, paths: _covers(12), disableAnimations: true),
      );
      final circles = state(tester).circles;
      final before = [
        for (var i = 0; i < circles.arrangement.slots.length; i += 1)
          circles.at(i),
      ];
      for (var i = 0; i < 60; i += 1) {
        await tester.pump(const Duration(seconds: 1));
        expect(state(tester).move, isNull);
        expect(tester.binding.transientCallbackCount, 0);
      }
      expect([
        for (var i = 0; i < circles.arrangement.slots.length; i += 1)
          circles.at(i),
      ], before);

      // It rolls once motion is allowed again, and a roll under way stops
      // when it no longer is.
      await tester.pumpWidget(app(proxy, paths: _covers(12)));
      await startRoll(tester);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpWidget(
        app(proxy, paths: _covers(12), disableAnimations: true),
      );
      expect(state(tester).move, isNull);
      await tester.pump(const Duration(seconds: 30));
      expect(state(tester).move, isNull);
      expect(tester.binding.transientCallbackCount, 0);
      await tester.pumpWidget(const SizedBox());
    });

    testWidgets('shows the covers of new paths', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      await tester.pumpWidget(app(proxy, paths: const ['x', 'y', 'z']));
      expect(_inSight(state(tester).circles).toSet(), {'x', 'y', 'z'});
      expect(proxy.requests.keys, containsAll(['x', 'y', 'z']));
      await startRoll(tester);
      await endRoll(tester);
      await tester.pumpWidget(const SizedBox());
    });

    for (final kind in CircleArrangementKind.values) {
      for (final cells in [1, 2]) {
        testWidgets('rolls the ${kind.name} arrangement on $cells cells and '
            'never shows a cover twice', (tester) async {
          final proxy = _ImageProxy();
          await tester.pumpWidget(
            app(proxy, paths: _covers(24), arrangement: kind, cells: cells),
          );
          final circles = state(tester).circles;
          final expected = CircleArrangement.of(kind, cells);
          expect(circles.arrangement.slots, hasLength(expected.slots.length));
          expect(circles.arrangement.fades, expected.fades);
          // Covers are decoded for the largest circle.
          expect(proxy.sizes, {(252 * expected.largestDiameter).ceil() * 2});
          for (var turn = 0; turn < 12; turn += 1) {
            await tester.pump(rollingRestMax);
            expect(state(tester).move ?? state(tester).fade, isNotNull);
            _expectNoCoverTwice(circles, 'turn $turn, under way');
            await tester.pump();
            await tester.pump(rollingTwoSlots);
            await tester.pump(rollingFade);
            await tester.pump(const Duration(milliseconds: 10));
            await tester.pump();
            expect(state(tester).move ?? state(tester).fade, isNull);
            expect(tester.binding.transientCallbackCount, 0);
            _expectNoCoverTwice(circles, 'turn $turn, at rest');
          }
          await tester.pumpWidget(const SizedBox());
        });
      }
    }

    testWidgets('fades the large circle out, changes its cover and fades it '
        'in', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(
        app(
          proxy,
          paths: _covers(12),
          arrangement: CircleArrangementKind.bigAndSmall,
          seed: 2,
        ),
      );
      final circles = state(tester).circles;
      final big = circles.arrangement.fades.single;
      final image = await _image();
      for (final request in proxy.requests.values) {
        request.complete(image);
      }
      await tester.pump();
      // Rolls go by until a turn is a fade.
      var turns = 0;
      while (state(tester).fade == null) {
        expect(turns += 1, lessThan(60));
        await tester.pump(rollingTwoSlots);
        await tester.pump(const Duration(milliseconds: 10));
        await tester.pump();
        for (final request in proxy.requests.values) {
          if (!request.isCompleted) request.complete(image);
        }
        await tester.pump(rollingRestMax);
      }
      final fade = state(tester).fade!;
      expect(fade.slot, big);
      final old = circles.at(big).cover;
      final next = circles.at(fade.from).cover;
      final inSight = _inSight(circles);
      expect(inSight, isNot(contains(next)));
      final size = tester.getSize(paint());
      final rects = painter(tester).circleRects(size);

      // Half way out it still has its cover, half as faint.
      await tester.pump();
      await tester.pump(rollingFade ~/ 2);
      expect(circles.at(big).cover, old);
      expect(painter(tester).progress.value, closeTo(0.5, 0.01));
      expect(painter(tester).circleRects(size), rects);

      // Out of sight it takes the new cover, and comes back with it.
      await tester.pump(rollingFade ~/ 2 + const Duration(milliseconds: 1));
      await tester.pump();
      expect(state(tester).fade, same(fade));
      expect(circles.at(big).cover, next);
      _expectNoCoverTwice(circles, 'faded out');
      await tester.pump(rollingFade ~/ 2);
      expect(painter(tester).progress.value, closeTo(0.5, 0.05));
      await tester.pump(rollingFade);
      await tester.pump();
      expect(state(tester).fade, isNull);
      expect(painter(tester).progress.value, 0);
      expect(tester.binding.transientCallbackCount, 0);
      expect(circles.at(big).cover, next);

      // Another fade, under way, stops with the tile.
      turns = 0;
      while (state(tester).fade == null) {
        expect(turns += 1, lessThan(60));
        await tester.pump(rollingRestMax);
        await tester.pump(rollingTwoSlots);
        await tester.pump(const Duration(milliseconds: 10));
        await tester.pump();
        for (final request in proxy.requests.values) {
          if (!request.isCompleted) request.complete(image);
        }
        await tester.pump(rollingRestMax);
      }
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      expect(state(tester).fade, isNotNull);
      expect(tester.binding.transientCallbackCount, 1);
      await tester.pumpWidget(const SizedBox());
      expect(tester.binding.transientCallbackCount, 0);
      await tester.pump(const Duration(seconds: 30));
      expect(tester.binding.transientCallbackCount, 0);
      image.dispose();
    });

    testWidgets('stops its rest timer when removed', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      await tester.pump(const Duration(seconds: 3));
      await tester.pumpWidget(const SizedBox());
      expect(tester.binding.transientCallbackCount, 0);
      // A pending timer would fail the test at its end.
    });

    testWidgets('stops a roll under way when removed', (tester) async {
      final proxy = _ImageProxy();
      await tester.pumpWidget(app(proxy));
      await startRoll(tester);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.binding.transientCallbackCount, 1);
      await tester.pumpWidget(const SizedBox());
      expect(tester.binding.transientCallbackCount, 0);
      await tester.pump(const Duration(seconds: 30));
      expect(tester.binding.transientCallbackCount, 0);
    });
  });
}
