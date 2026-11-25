import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/router/router_aware_flyout_controller.dart';
import 'package:rune/widgets/artist_page/release_arrangements.dart';
import 'package:rune/widgets/artist_page/release_spans.dart';
import 'package:rune/widgets/artist_page/release_tile.dart';
import 'package:rune/widgets/ax_pressure.dart';
import 'package:rune/widgets/ax_reveal/ax_reveal.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/context_menu_wrapper.dart';
import 'package:rune/widgets/tile/fast_flip_cover_grid.dart';
import 'package:rune/widgets/tile/flip_tile.dart';
import 'package:rune/widgets/tile/rolling_cover_circles.dart';
import 'package:rune/widgets/tile/tile.dart';
import 'package:rune/widgets/tile_group/tile_layout.dart';

ScreenSizeProvider? _sizes;

/// [child] in the middle of a window, with the providers a tile reads the
/// window's class from. They are made once per test.
Future<Widget> _app(
  WidgetTester tester,
  Widget child, {
  bool disableAnimations = false,
}) async {
  var sizes = _sizes;
  if (sizes == null) {
    sizes = _sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    addTearDown(() {
      _sizes?.dispose();
      _sizes = null;
    });
    // The size provider saves the size after a second.
    await tester.pump(const Duration(seconds: 1));
  }
  return MultiProvider(
    providers: [
      ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
      ChangeNotifierProvider<ResponsiveProvider>(
        create: (_) => ResponsiveProvider(_sizes!, () async => null),
      ),
    ],
    child: FluentApp(
      builder: (context, child) => RevealEffectContext(child: child!),
      home: MediaQuery(
        data: MediaQueryData(
          size: const Size(800, 800),
          disableAnimations: disableAnimations,
        ),
        child: Center(child: child),
      ),
    ),
  );
}

/// A tile in a box of its size, as a tile group gives it one. It has more
/// covers than circles in sight unless told otherwise, so that a roll always
/// has a cover to bring in.
Widget _tile({
  List<String> covers = const [
    'a',
    'b',
    'c',
    'd',
    'e',
    'f',
    'g',
    'h',
    'i',
    'j',
    'k',
    'l',
  ],
  int cells = 2,
  CircleArrangementKind arrangement = CircleArrangementKind.square,
  VoidCallback? onPressed,
  ReleaseContextMenuOpener? onContextMenu,
  VoidCallback? onMiddleClick,
}) {
  return SizedBox(
    width: cells * 120,
    height: cells * 120,
    child: ReleaseTile(
      onPressed: onPressed ?? () {},
      onContextMenu: onContextMenu,
      onMiddleClick: onMiddleClick,
      label: 'Albums',
      covers: covers,
      cells: cells,
      arrangement: arrangement,
    ),
  );
}

/// Removes the tile and lets the image cache clear itself, which it does a
/// moment after the last tile with covers is gone.
Future<void> _remove(WidgetTester tester) async {
  await tester.pumpWidget(const SizedBox());
  await tester.pump(const Duration(milliseconds: 200));
}

RollingCoverCircles _circles(WidgetTester tester) =>
    tester.widget<RollingCoverCircles>(find.byType(RollingCoverCircles));

RollingCoverCirclesState _state(WidgetTester tester) =>
    tester.state(find.byType(RollingCoverCircles));

/// The covers on the circles in sight.
List<String> _inSight(WidgetTester tester) => [
  for (final circle in _state(tester).circles.circlesInSight)
    if (circle.cover != null) circle.cover!,
];

void main() {
  group('release spans', () {
    /// The cells the tiles take when a tile group places them with strip 1,
    /// each as its place along the group and its place on the short side.
    Set<(int, int)> cells(List<TileSpan> spans, TileLayout layout, Axis axis) {
      final seen = <(int, int)>{};
      for (final (i, p) in layout.placements.indexed) {
        for (var c = p.column; c < p.column + spans[i].columns; c += 1) {
          for (var r = p.row; r < p.row + spans[i].rows; r += 1) {
            final cell = axis == Axis.horizontal ? (c, r) : (r, c);
            expect(seen.add(cell), isTrue, reason: '$p overlaps');
          }
        }
      }
      return seen;
    }

    /// The layouts of the spans in a group that grows sideways and in one
    /// that grows downward, with the lines in use and the cells taken.
    List<({int lines, int across, Set<(int, int)> cells})> layouts(
      List<TileSpan> spans,
      int short,
    ) {
      final across = layoutTiles(spans, rows: short);
      final down = layoutTiles(
        spans,
        orientation: Axis.vertical,
        columns: short,
      );
      return [
        (
          lines: across.columns,
          across: across.rows,
          cells: cells(spans, across, Axis.horizontal),
        ),
        (
          lines: down.rows,
          across: down.columns,
          cells: cells(spans, down, Axis.vertical),
        ),
      ];
    }

    for (final short in [1, 2, 3]) {
      for (var count = 1; count <= 12; count += 1) {
        test('short side $short with $count sections has square tiles and no '
            'holes', () {
          final spans = releaseSpans(count, short, largeReleaseTileCovers);
          expect(spans, hasLength(count));
          for (final span in spans) {
            expect(
              (span.columns, span.rows),
              anyOf((1, 1), (2, 2)),
              reason: 'a tile is one cell or two by two',
            );
            expect(span.columns, lessThanOrEqualTo(short));
          }
          // One section on a short side of three is a single two by two
          // tile, as drawn in the design.
          final single = count == 1 && short == 3;
          for (final layout in layouts(spans, short)) {
            expect(layout.across, single ? 2 : short);
            // Along the order the group fills its cells in, no free cell
            // comes before a tile: the free cells end the last line.
            final order = [
              for (var line = 0; line < layout.lines; line += 1)
                for (var at = 0; at < layout.across; at += 1) (line, at),
            ];
            expect(
              layout.cells.difference(order.toSet()),
              isEmpty,
              reason: 'a tile lies outside the reported rectangle',
            );
            final taken = order.lastIndexWhere(layout.cells.contains) + 1;
            expect(
              order.take(taken).where((x) => !layout.cells.contains(x)),
              isEmpty,
              reason: 'holes',
            );
            final free = order.length - taken;
            // The tiles fill the block to its end when the short side divides
            // the cells they take: one large tile of four and one cell for
            // every other section.
            final used = short == 1 ? count : count + 3;
            if (used % short == 0) {
              expect(free, 0, reason: 'the block is not filled');
            }
            expect(free, lessThan(short));
          }
        });
      }
    }

    test('the first section takes a large tile where the group has room and '
        'the section has covers for it', () {
      for (var count = 1; count <= 12; count += 1) {
        for (final covers in [6, 7, 20, 60]) {
          expect(releaseSpans(count, 1, covers), everyElement(_isSpan(1)));
          for (final short in [2, 3]) {
            final spans = releaseSpans(count, short, covers);
            expect(spans.first, _isSpan(2), reason: '$count on $short');
          }
        }
      }
    });

    test('every section is one cell when the first has too few covers', () {
      for (var count = 1; count <= 12; count += 1) {
        for (var covers = 0; covers < 6; covers += 1) {
          for (final short in [1, 2, 3]) {
            expect(
              releaseSpans(count, short, covers),
              everyElement(_isSpan(1)),
              reason: '$count on $short with $covers covers',
            );
          }
        }
      }
    });

    test('a large tile asks for the covers of the big and small '
        'arrangement', () {
      expect(
        largeReleaseTileCovers,
        CircleArrangement.of(CircleArrangementKind.bigAndSmall, 2).inSight,
      );
    });

    test('only the first section is large', () {
      for (final short in [2, 3]) {
        for (var count = 1; count <= 12; count += 1) {
          expect(
            [for (final s in releaseSpans(count, short, 6)) s.columns * s.rows],
            [4, for (var i = 1; i < count; i += 1) 1],
            reason: '$count on $short',
          );
        }
      }
      expect(releaseSpans(0, 3, 0), isEmpty);
    });
  });

  group('release arrangements', () {
    const big = CircleArrangementKind.bigAndSmall;

    /// The arrangements of [count] tiles, the first of two by two cells when
    /// [large], each of a section with [covers] covers.
    List<CircleArrangementKind> kinds(
      ReleaseArrangements arrangements,
      int count, {
      required bool large,
      int covers = 60,
    }) => [
      for (var i = 0; i < count; i += 1)
        arrangements.of(i, large && i == 0 ? 2 : 1, covers),
    ];

    int inSight(CircleArrangementKind kind, int cells) =>
        CircleArrangement.of(kind, cells).inSight;

    test('differ on every tile of one cell while there are enough, and the '
        'large one may be any', () {
      final largeKinds = <CircleArrangementKind>{};
      for (var seed = 0; seed < 60; seed += 1) {
        final arrangements = ReleaseArrangements(Random(seed));
        for (var count = 1; count <= 4; count += 1) {
          final picked = kinds(arrangements, count, large: true);
          largeKinds.add(picked.first);
          expect(
            picked.skip(1).toSet(),
            hasLength(count - 1),
            reason: '$seed, $count',
          );
        }
        for (var count = 1; count <= 3; count += 1) {
          final picked = kinds(arrangements, count, large: false);
          expect(picked.toSet(), hasLength(count), reason: '$seed, $count');
        }
      }
      expect(largeKinds, CircleArrangementKind.values.toSet());
    });

    test('never put the big and small one on a tile of one cell', () {
      for (var seed = 0; seed < 60; seed += 1) {
        final arrangements = ReleaseArrangements(Random(seed));
        for (var count = 1; count <= 12; count += 1) {
          expect(
            kinds(arrangements, count, large: false),
            isNot(contains(big)),
          );
          expect(
            kinds(arrangements, count, large: true).skip(1),
            isNot(contains(big)),
          );
        }
      }
    });

    test('come again evenly when the tiles outnumber them', () {
      for (var seed = 0; seed < 60; seed += 1) {
        final arrangements = ReleaseArrangements(Random(seed));
        for (var count = 4; count <= 12; count += 1) {
          for (final large in [true, false]) {
            // The tiles of one cell; the large one is free to be any.
            final picked = kinds(arrangements, count, large: large);
            final small = large ? picked.skip(1).toList() : picked;
            final uses = [
              for (final kind in small.toSet())
                small.where((x) => x == kind).length,
            ];
            expect(small.toSet(), hasLength(3));
            expect(
              uses.reduce(max) - uses.reduce(min),
              lessThanOrEqualTo(1),
              reason: '$seed, $count',
            );
            // Next to each other no two tiles of one cell are alike.
            for (var i = large ? 2 : 1; i < count; i += 1) {
              expect(picked[i], isNot(picked[i - 1]));
            }
          }
        }
      }
    });

    test('stay as picked, and when the first tile changes its size', () {
      final arrangements = ReleaseArrangements(Random(4));
      for (var count = 1; count <= 8; count += 1) {
        final large = kinds(arrangements, count, large: true);
        final small = kinds(arrangements, count, large: false);
        expect(kinds(arrangements, count, large: true), large);
        expect(kinds(arrangements, count, large: false), small);
        expect(large.skip(1), small.skip(1));
      }
    });

    test('are picked anew for another page', () {
      final picked = {
        for (var seed = 0; seed < 60; seed += 1)
          kinds(ReleaseArrangements(Random(seed)), 3, large: true).join(),
      };
      expect(picked.length, greaterThan(6));
      // Every arrangement can be the large tile's and a small tile's.
      final first = {
        for (var seed = 0; seed < 60; seed += 1)
          ReleaseArrangements(Random(seed)).of(0, 2, 60),
      };
      expect(first, CircleArrangementKind.values.toSet());
      final second = {
        for (var seed = 0; seed < 60; seed += 1)
          ReleaseArrangements(Random(seed)).of(1, 1, 60),
      };
      expect(second, CircleArrangementKind.values.toSet()..remove(big));
    });

    test('are the one with the fewest circles in sight for one cover', () {
      for (final cells in [1, 2]) {
        final fewest = CircleArrangementKind.values
            .map((kind) => inSight(kind, cells))
            .reduce(min);
        for (var seed = 0; seed < 60; seed += 1) {
          final arrangements = ReleaseArrangements(Random(seed));
          for (var index = 0; index < (cells == 1 ? 12 : 1); index += 1) {
            for (final covers in [0, 1]) {
              final kind = arrangements.of(index, cells, covers);
              expect(inSight(kind, cells), fewest, reason: '$seed, $index');
              if (cells == 1) expect(kind, isNot(big));
            }
          }
        }
      }
    });

    test('have no more circles in sight than the section has covers', () {
      for (final cells in [1, 2]) {
        final counts = CircleArrangementKind.values
            .where((kind) => cells == 2 || kind != big)
            .map((kind) => inSight(kind, cells))
            .toSet();
        final fewest = counts.reduce(min);
        for (var covers = fewest; covers <= 24; covers += 1) {
          final picked = <CircleArrangementKind>{};
          for (var seed = 0; seed < 60; seed += 1) {
            final arrangements = ReleaseArrangements(Random(seed));
            for (var index = 0; index < (cells == 1 ? 6 : 1); index += 1) {
              picked.add(arrangements.of(index, cells, covers));
            }
          }
          // Every arrangement the covers can fill is picked on some page,
          // and no other.
          expect(
            picked.map((kind) => inSight(kind, cells)).toSet(),
            counts.where((count) => count <= covers).toSet(),
            reason: '$covers covers on $cells cells',
          );
        }
      }
    });

    test('give the large tile the hex one only from twenty covers', () {
      const hex = CircleArrangementKind.hex;
      expect(inSight(hex, 2), 20);
      Set<CircleArrangementKind> large(int covers) => {
        for (var seed = 0; seed < 60; seed += 1)
          ReleaseArrangements(Random(seed)).of(0, 2, covers),
      };
      expect(large(19), CircleArrangementKind.values.toSet()..remove(hex));
      expect(large(20), CircleArrangementKind.values.toSet());
    });

    test('stay spread over the tiles of one cell that can take several', () {
      // Enough covers for two of the three arrangements of one cell.
      final counts = [
        for (final kind in CircleArrangementKind.values)
          if (kind != big) inSight(kind, 1),
      ]..sort();
      final covers = counts[1];
      expect(covers, lessThan(counts[2]));
      for (var seed = 0; seed < 60; seed += 1) {
        final arrangements = ReleaseArrangements(Random(seed));
        final picked = kinds(arrangements, 12, large: false, covers: covers);
        final uses = [
          for (final kind in picked.toSet())
            picked.where((x) => x == kind).length,
        ];
        expect(uses, [6, 6], reason: '$seed');
        // Next to each other no two tiles are alike.
        for (var i = 1; i < 12; i += 1) {
          expect(picked[i], isNot(picked[i - 1]), reason: '$seed');
        }
      }
    });
  });

  group('release tile', () {
    for (final (cells, side) in const [(1, 2), (2, 3)]) {
      testWidgets('a tile of $cells cells a side has $side circles a side', (
        tester,
      ) async {
        await tester.pumpWidget(await _app(tester, _tile(cells: cells)));
        expect(_circles(tester).cells, cells);
        expect(_circles(tester).arrangement, CircleArrangementKind.square);
        expect(_state(tester).circles.circlesInSight, hasLength(side * side));
        expect(find.byType(FastFlipCoverGrid), findsNothing);
        expect(find.byType(EmptyFlipCover), findsNothing);
        await _remove(tester);
      });
    }

    for (final kind in CircleArrangementKind.values) {
      testWidgets('lays its circles out as ${kind.name} when told to', (
        tester,
      ) async {
        await tester.pumpWidget(await _app(tester, _tile(arrangement: kind)));
        expect(_circles(tester).arrangement, kind);
        expect(
          _state(tester).circles.arrangement.slots,
          hasLength(CircleArrangement.of(kind, 2).slots.length),
        );
        await _remove(tester);
      });
    }

    for (final count in const [1, 3, 8, 9, 12]) {
      testWidgets('$count covers are in sight first, each once', (
        tester,
      ) async {
        final covers = [for (var i = 0; i < count; i += 1) 'c$i'];
        await tester.pumpWidget(await _app(tester, _tile(covers: covers)));
        expect(_circles(tester).paths, covers);
        final inSight = _inSight(tester);
        expect(inSight, hasLength(count < 9 ? count : 9));
        expect(inSight.toSet(), hasLength(inSight.length));
        expect(covers, containsAll(inSight));
        await _remove(tester);
      });
    }

    testWidgets('duplicate covers count once', (tester) async {
      await tester.pumpWidget(
        await _app(tester, _tile(covers: const ['a', 'b', 'a', 'a', 'b'])),
      );
      expect(_circles(tester).paths, ['a', 'b']);
      expect(_inSight(tester).toSet(), {'a', 'b'});
      expect(_inSight(tester), hasLength(2));
      await _remove(tester);
    });

    testWidgets('covers are decoded for the size the tile is laid out at', (
      tester,
    ) async {
      Widget boxed(double side) => SizedBox(
        width: side,
        height: side,
        child: ReleaseTile(
          label: 'Albums',
          covers: const ['a', 'b', 'c'],
          cells: 1,
          onPressed: () {},
        ),
      );
      // The cell of the turntile screen is smaller than that of the home.
      for (final side in [88.0, 120.0, 252.0]) {
        await tester.pumpWidget(await _app(tester, boxed(side)));
        // The tile's border takes a pixel on each side.
        expect(_circles(tester).size, side.round() - 2);
      }
      await _remove(tester);
    });

    testWidgets('the circles lie on the accent colour, and those without a '
        'cover are shades of it', (tester) async {
      await tester.pumpWidget(
        await _app(tester, _tile(covers: const ['a', 'b'])),
      );
      final accent = FluentTheme.of(
        tester.element(find.byType(RollingCoverCircles)),
      ).accentColor;
      final circles = _circles(tester);
      expect(circles.background, accent);
      expect(circles.fillers, [
        accent.light,
        accent.lighter,
        accent.dark,
        accent.darker,
      ]);
      expect(circles.fillers, isNot(contains(circles.background)));
      // Seven of the nine circles in sight have no cover.
      final paint = find.descendant(
        of: find.byType(RollingCoverCircles),
        matching: find.byType(CustomPaint),
      );
      expect(paint, paints..rect(color: accent));
      expect(paint, paintsExactlyCountTimes(#drawCircle, 7));
      await _remove(tester);
    });

    testWidgets('the circles roll, and no cover is in sight twice', (
      tester,
    ) async {
      await tester.pumpWidget(await _app(tester, _tile()));
      var rolls = 0;
      var rolling = false;
      for (var i = 0; i < 600; i += 1) {
        await tester.pump(const Duration(milliseconds: 100));
        final now = _state(tester).move != null;
        if (now && !rolling) rolls += 1;
        rolling = now;
        final inSight = _inSight(tester);
        expect(inSight.toSet(), hasLength(inSight.length));
        // While a lane rolls its circles are on the slots they left.
        expect(inSight, hasLength(9));
      }
      // A minute holds five rolls at the least, each after a rest of six to
      // eleven seconds.
      expect(rolls, inInclusiveRange(5, 10));
      await _remove(tester);
    });

    testWidgets('the circles are still under reduced motion', (tester) async {
      await tester.pumpWidget(
        await _app(tester, _tile(), disableAnimations: true),
      );
      final before = _inSight(tester);
      for (var i = 0; i < 60; i += 1) {
        await tester.pump(const Duration(seconds: 1));
        expect(_state(tester).move, isNull);
      }
      expect(tester.binding.transientCallbackCount, 0);
      expect(_inSight(tester), before);
      await _remove(tester);
    });

    testWidgets('an empty cover list shows the empty tile and never rolls', (
      tester,
    ) async {
      await tester.pumpWidget(await _app(tester, _tile(covers: const [])));
      expect(find.byType(RollingCoverCircles), findsNothing);
      final empty = tester.widget<EmptyFlipCover>(find.byType(EmptyFlipCover));
      // The empty tile of a collection tile.
      final context = tester.element(find.byType(EmptyFlipCover));
      expect(empty.name, 'Albums');
      expect(empty.colors, flipTileColors(FluentTheme.of(context)));
      expect(
        empty.emptyTileType,
        FlipTile(name: '', paths: const [], onPressed: () {}).emptyTileType,
      );
      expect(find.text('Albums'), findsOneWidget);
      await tester.pump(const Duration(seconds: 30));
      expect(tester.hasRunningAnimations, isFalse);
      await _remove(tester);
    });

    testWidgets("the label is drawn as a collection tile's name", (
      tester,
    ) async {
      await tester.pumpWidget(await _app(tester, _tile()));
      final cover = tester.widget<NamedTileCover>(find.byType(NamedTileCover));
      expect(cover.name, 'Albums');
      expect(find.text('Albums'), findsOneWidget);
      await _remove(tester);
    });

    testWidgets('a band window draws the covers alone, as a collection tile', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(640, 130);
      addTearDown(tester.view.reset);
      final semantics = tester.ensureSemantics();
      await tester.pumpWidget(await _app(tester, _tile(cells: 1)));
      expect(find.byType(RollingCoverCircles), findsOneWidget);
      expect(find.text('Albums'), findsNothing);
      // The name is not drawn, but it is the tile's name to a screen reader.
      expect(find.bySemanticsLabel('Albums'), findsOneWidget);
      await _remove(tester);
      semantics.dispose();
    });

    testWidgets('is the shell of a collection tile around its covers', (
      tester,
    ) async {
      await tester.pumpWidget(await _app(tester, _tile()));
      // The same wrappers, in the same order, as CollectionItem puts around
      // its FlipTile, which is a Tile around a NamedTileCover.
      final shell = [
        AxPressure,
        AxReveal,
        ContextMenuWrapper,
        Tile,
        NamedTileCover,
        RollingCoverCircles,
      ];
      var previous = -1;
      for (final type in shell) {
        final found = find.byType(type);
        expect(found, findsOneWidget, reason: '$type');
        final depth = tester.elementList(found).single.depth;
        expect(depth, greaterThan(previous), reason: '$type');
        previous = depth;
      }
      await _remove(tester);
    });

    testWidgets('a press opens after the tile\'s short delay', (tester) async {
      var pressed = 0;
      await tester.pumpWidget(
        await _app(tester, _tile(onPressed: () => pressed += 1)),
      );
      await tester.tap(find.byType(Tile));
      expect(pressed, 0);
      await tester.pump(const Duration(milliseconds: 130));
      expect(pressed, 1);
      await _remove(tester);
    });

    testWidgets('a right click opens the context menu on the tile', (
      tester,
    ) async {
      final opened = <(Offset, GlobalKey, RouterAwareFlyoutController)>[];
      await tester.pumpWidget(
        await _app(
          tester,
          _tile(
            onContextMenu: (context, position, key, controller) =>
                opened.add((position, key, controller)),
          ),
        ),
      );
      final gesture = await tester.startGesture(
        tester.getCenter(find.byType(Tile)),
        kind: PointerDeviceKind.mouse,
        buttons: kSecondaryMouseButton,
      );
      await gesture.up();
      await tester.pump();
      expect(opened, hasLength(1));
      // The menu hangs on the wrapper's own target.
      final wrapper = tester.widget<ContextMenuWrapper>(
        find.byType(ContextMenuWrapper),
      );
      expect(opened.single.$2, wrapper.contextAttachKey);
      expect(opened.single.$3, wrapper.contextController);
      await _remove(tester);
    });

    testWidgets(
      'a long press opens the context menu on a touch device',
      (tester) async {
        var opened = 0;
        await tester.pumpWidget(
          await _app(tester, _tile(onContextMenu: (_, _, _, _) => opened += 1)),
        );
        await tester.longPress(find.byType(Tile));
        expect(opened, 1);
        await _remove(tester);
      },
      variant: const TargetPlatformVariant({
        TargetPlatform.android,
        TargetPlatform.iOS,
      }),
    );

    testWidgets('a middle click runs the middle click action', (tester) async {
      var clicked = 0;
      await tester.pumpWidget(
        await _app(tester, _tile(onMiddleClick: () => clicked += 1)),
      );
      final gesture = await tester.startGesture(
        tester.getCenter(find.byType(Tile)),
        kind: PointerDeviceKind.mouse,
        buttons: kMiddleMouseButton,
      );
      await gesture.up();
      await tester.pump();
      expect(clicked, 1);
      await _remove(tester);
    });

    testWidgets('removing the tile stops its rolling', (tester) async {
      await tester.pumpWidget(await _app(tester, _tile()));
      // At rest.
      await tester.pump(const Duration(seconds: 3));
      await _remove(tester);
      expect(tester.binding.transientCallbackCount, 0);

      // In the middle of a roll.
      await tester.pumpWidget(await _app(tester, _tile()));
      await tester.pump(rollingRestMax);
      expect(_state(tester).move, isNotNull);
      await tester.pump(const Duration(milliseconds: 100));
      expect(_state(tester).move, isNotNull);
      await _remove(tester);
      expect(tester.binding.transientCallbackCount, 0);
      // A pending timer would fail the test at its end.
    });
  });
}

/// Matches a square tile with [side] cells along a side.
Matcher _isSpan(int side) => isA<TileSpan>()
    .having((s) => s.columns, 'columns', side)
    .having((s) => s.rows, 'rows', side);
