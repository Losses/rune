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
          tileSpanOf: (item) => _spans[item],
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
            tileSpanOf: spanOf,
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
      tileSpanOf: spanOf,
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
}
