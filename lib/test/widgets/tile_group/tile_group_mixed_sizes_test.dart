import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/widgets/start_screen/managed_start_screen_item.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/start_screen/start_group.dart';
import 'package:rune/widgets/tile_group/tile_grid.dart';
import 'package:rune/widgets/tile_group/tile_layout.dart';
import 'package:rune/widgets/turntile/managed_turntile_screen_item.dart';
import 'package:rune/widgets/turntile/turntile_group.dart';
import 'package:rune/widgets/turntile/turntile_group_items_tile.dart';

const _spans = [
  TileSpan(2, 2),
  TileSpan(1, 1),
  TileSpan(2, 1),
  TileSpan(1, 2),
  TileSpan(1, 1),
  TileSpan(1, 1),
];

const _gap = 4.0;

const _largeRects = [
  Rect.fromLTWH(0, 0, 244, 244),
  Rect.fromLTWH(0, 248, 120, 120),
  Rect.fromLTWH(124, 248, 244, 120),
  Rect.fromLTWH(248, 0, 120, 244),
  Rect.fromLTWH(372, 0, 120, 120),
  Rect.fromLTWH(372, 124, 120, 120),
];

const _smallRects = [
  Rect.fromLTWH(0, 0, 180, 180),
  Rect.fromLTWH(184, 0, 88, 88),
  Rect.fromLTWH(184, 92, 180, 88),
  Rect.fromLTWH(0, 184, 88, 180),
  Rect.fromLTWH(92, 184, 88, 88),
  Rect.fromLTWH(184, 184, 88, 88),
];

Widget _host(Widget child) {
  return FluentTheme(
    data: FluentThemeData(),
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: StartScreenLayoutManager(),
        child: Align(alignment: Alignment.topLeft, child: child),
      ),
    ),
  );
}

Rect _relativeRect(WidgetTester tester, Finder finder, Finder grid) {
  final origin = tester.getTopLeft(grid);
  return tester.getRect(finder).shift(-origin);
}

void main() {
  testWidgets('Large group places tiles of mixed sizes by the layout', (
    tester,
  ) async {
    await tester.pumpWidget(
      _host(
        StartGroup<int>(
          groupIndex: 0,
          groupTitle: 'Group',
          items: const [0, 1, 2, 3, 4, 5],
          itemBuilder: (context, item) => const SizedBox.expand(),
          constraints: const BoxConstraints(maxWidth: 2000, maxHeight: 396),
          gapSize: _gap,
          direction: Axis.vertical,
          tileSpanBuilder: (item) => _spans[item],
        ),
      ),
    );

    final layout = layoutTiles(_spans, rows: 3);
    final grid = find.byType(TileGrid);
    final managed = find.byType(ManagedStartScreenItem);
    expect(managed, findsNWidgets(_spans.length));

    for (var i = 0; i < _spans.length; i++) {
      expect(
        _relativeRect(tester, managed.at(i), grid),
        _largeRects[i],
        reason: 'tile $i',
      );
      final item = tester.widget<ManagedStartScreenItem>(managed.at(i));
      expect(item.row, layout.placements[i].row, reason: 'row of tile $i');
      expect(
        item.column,
        layout.placements[i].column,
        reason: 'column of tile $i',
      );
    }

    expect(
      TileGrid.packedSize(layout, defaultCellSize, _gap),
      const Size(492, 368),
    );
    expect(tester.getSize(grid), const Size(492, 368));
  });

  testWidgets('Small group places tiles of mixed sizes by the layout', (
    tester,
  ) async {
    const items = [0, 1, 2, 3, 4, 5];
    TileSpan spanOf(int item) => _spans[item];

    await tester.pumpWidget(
      _host(
        SizedBox(
          width: 400,
          child: TurntileGroup<int>(
            groupIndex: 0,
            groupTitle: 'Group',
            items: items,
            itemBuilder: (context, item) => const SizedBox.expand(),
            gapSize: _gap,
            gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
            tileSpanBuilder: spanOf,
          ),
        ),
      ),
    );

    // 400 wide minus the padding of the group on both sides.
    final measured = TurntileGroupItemsTile<int>(
      cellSize: 88,
      gapSize: _gap,
      items: items,
      groupIndex: 0,
      itemBuilder: (context, item) => const SizedBox.expand(),
      tileSpanBuilder: spanOf,
    ).measure(368);
    final layout = layoutTiles(_spans, orientation: Axis.vertical, columns: 4);
    final grid = find.byType(TileGrid);
    final managed = find.byType(ManagedTurntileScreenItem);
    expect(managed, findsNWidgets(_spans.length));

    for (var i = 0; i < _spans.length; i++) {
      expect(
        _relativeRect(tester, managed.at(i), grid),
        _smallRects[i],
        reason: 'tile $i',
      );
      final item = tester.widget<ManagedTurntileScreenItem>(managed.at(i));
      expect(item.row, layout.placements[i].row, reason: 'row of tile $i');
      expect(
        item.column,
        layout.placements[i].column,
        reason: 'column of tile $i',
      );
    }

    expect(tester.getSize(grid), const Size(368, 364));
    expect(measured.size, const Size(368, 364));
    expect(measured.cellSize, 88);
  });

  test(
    'A group ending in a 2x2 tile counts its full width for the next group',
    () {
      double distanceAfter(int columns, int rows) {
        final manager = StartScreenLayoutManager();
        void noop() {}
        manager.registerItem(0, 0, 0, noop);
        manager.registerItem(0, 0, 1, noop);
        manager.registerItem(0, 0, 2, noop, null, columns, rows);
        return manager.registerItem(1, 0, 0, noop).data!.distance;
      }

      // Group 0 is 3 columns and 2 rows wide: the offset is (3, 1).
      expect(distanceAfter(2, 2), closeTo(3.1623, 0.0001));
      // The same top-left cell with a 1x1 tile: 2 columns, 1 row.
      expect(distanceAfter(1, 1), 2.0);
    },
  );

  testWidgets('Square group with 2x2 tiles stays within the block columns', (
    tester,
  ) async {
    Future<List<ManagedStartScreenItem>> shown(
      TileSpan Function(int item) spanBuilder,
    ) async {
      await tester.pumpWidget(
        _host(
          StartGroup<int>(
            groupIndex: 0,
            groupTitle: 'Group',
            items: List.generate(20, (i) => i),
            itemBuilder: (context, item) => const SizedBox.expand(),
            constraints: const BoxConstraints(maxWidth: 2000, maxHeight: 528),
            gapSize: _gap,
            direction: Axis.vertical,
            gridLayoutVariation: StartGroupGridLayoutVariation.square,
            tileSpanBuilder: spanBuilder,
          ),
        ),
      );
      return tester
          .widgetList<ManagedStartScreenItem>(
            find.byType(ManagedStartScreenItem),
          )
          .toList();
    }

    // 528 high gives a block of 4 x 4 cells.
    final plain = await shown((item) => const TileSpan(1, 1));
    expect(plain.length, 16);

    final mixed = await shown(
      (item) => item % 4 == 0 ? const TileSpan(2, 2) : const TileSpan(1, 1),
    );
    expect(mixed.length, 8);
    for (final item in mixed) {
      expect(item.column + item.columns, lessThanOrEqualTo(4));
    }
  });

  testWidgets('Small group fills bands of two rows with strip 2', (
    tester,
  ) async {
    // A medium tile and four small ones, 400 wide: 4 columns of 88.
    TileSpan spanOf(int item) =>
        item == 0 ? const TileSpan(2, 2) : const TileSpan(1, 1);

    Future<List<Offset>> smallTiles(int strip) async {
      await tester.pumpWidget(
        _host(
          SizedBox(
            width: 400,
            child: TurntileGroup<int>(
              groupIndex: 0,
              groupTitle: 'Group',
              items: const [0, 1, 2, 3, 4],
              itemBuilder: (context, item) => const SizedBox.expand(),
              gapSize: _gap,
              gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
              tileSpanBuilder: spanOf,
              strip: strip,
            ),
          ),
        ),
      );
      final grid = find.byType(TileGrid);
      final managed = find.byType(ManagedTurntileScreenItem);
      return [
        for (var i = 1; i < 5; i++)
          _relativeRect(tester, managed.at(i), grid).topLeft,
      ];
    }

    // Row by row: the small tiles go left to right, then down.
    expect(await smallTiles(1), const [
      Offset(184, 0),
      Offset(276, 0),
      Offset(184, 92),
      Offset(276, 92),
    ]);
    // Bands of two rows: the small tiles go top to bottom, then right.
    expect(await smallTiles(2), const [
      Offset(184, 0),
      Offset(184, 92),
      Offset(276, 0),
      Offset(276, 92),
    ]);
  });
}
