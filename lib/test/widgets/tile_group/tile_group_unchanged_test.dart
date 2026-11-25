import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/widgets/start_screen/managed_start_screen_item.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/start_screen/start_group.dart';
import 'package:rune/widgets/start_screen/start_group_implementation.dart';
import 'package:rune/widgets/start_screen/start_group_item.dart';
import 'package:rune/widgets/tile_group/tile_grid.dart';
import 'package:rune/widgets/turntile/managed_turntile_screen_item.dart';
import 'package:rune/widgets/turntile/turntile_group.dart';

// Groups of 1x1 tiles must stay where the former Wrap based groups put them.
// The expected values below are the placements of that former Wrap layout and
// are written out here on purpose, not derived from the tile layout code.

const _itemCounts = [1, 2, 5, 9, 13, 40];
const _largeHeights = [200.0, 396.0, 700.0];
const _smallListWidths = [150.0, 400.0, 731.5];
const _tolerance = 1e-9;

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

void _bigView(WidgetTester tester) {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(30000, 10000);
  addTearDown(tester.view.reset);
}

class _Variant {
  const _Variant(
    this.name,
    this.variation,
    this.calculator,
    this.direction,
    this.gap,
  );

  final String name;
  final StartGroupGridLayoutVariation variation;
  final Dimensions Function(double, double, double, List)? calculator;
  final Axis direction;
  final double gap;
}

// The three large group variants used on real pages.
final _variants = [
  // Collection pages.
  const _Variant(
    'default calculator, vertical',
    StartGroupGridLayoutVariation.initial,
    null,
    Axis.vertical,
    4,
  ),
  // Home links of the library and settings pages.
  _Variant(
    'start link calculator, horizontal',
    StartGroupGridLayoutVariation.initial,
    StartGroupImplementation.startLinkDimensionCalculator,
    Axis.horizontal,
    12,
  ),
  // Collections on the library home.
  const _Variant(
    'square, horizontal',
    StartGroupGridLayoutVariation.square,
    null,
    Axis.horizontal,
    12,
  ),
];

Dimensions _dimensions(_Variant v, double maxHeight, List items) {
  const cell = 120.0;
  switch (v.variation) {
    case StartGroupGridLayoutVariation.square:
      return StartGroupImplementation.squareDimensionCalculator(
        maxHeight,
        cell,
        v.gap,
        items,
      );
    case StartGroupGridLayoutVariation.initial:
      return (v.calculator ??
          StartGroupImplementation.defaultDimensionCalculator)(
        maxHeight,
        cell,
        v.gap,
        items,
      );
  }
}

void main() {
  for (final v in _variants) {
    for (final maxHeight in _largeHeights) {
      for (final count in _itemCounts) {
        testWidgets('Large group (${v.name}) of $count tiles in $maxHeight '
            'stays where the Wrap put them', (tester) async {
          _bigView(tester);
          final items = List<int>.generate(count, (i) => i);
          await tester.pumpWidget(
            _host(
              StartGroup<int>(
                groupIndex: 0,
                groupTitle: 'Group',
                items: items,
                itemBuilder: (context, item) => const SizedBox.expand(),
                constraints: BoxConstraints(
                  maxWidth: 30000,
                  maxHeight: maxHeight,
                ),
                gapSize: v.gap,
                direction: v.direction,
                gridLayoutVariation: v.variation,
                dimensionCalculator: v.calculator,
              ),
            ),
          );

          const cell = 120.0;
          final step = cell + v.gap;
          final d = _dimensions(v, maxHeight, items);
          final grid = find.byType(TileGrid);
          final managed = find.byType(ManagedStartScreenItem);
          expect(managed, findsNWidgets(d.count));

          final origin = tester.getTopLeft(grid);
          for (var i = 0; i < d.count; i++) {
            final int row;
            final int column;
            if (v.direction == Axis.horizontal) {
              row = i ~/ d.columns;
              column = i % d.columns;
            } else {
              row = i % d.rows;
              column = i ~/ d.rows;
            }
            final rect = tester.getRect(managed.at(i)).shift(-origin);
            expect(
              rect,
              Rect.fromLTWH(column * step, row * step, cell, cell),
              reason: 'tile $i',
            );
            final widget = tester.widget<ManagedStartScreenItem>(managed.at(i));
            expect(widget.row, row, reason: 'row of tile $i');
            expect(widget.column, column, reason: 'column of tile $i');
          }

          expect(
            tester.getSize(grid),
            Size(
              max(0.0, d.columns * step - v.gap),
              max(0.0, d.rows * step - v.gap),
            ),
          );
        });
      }
    }
  }

  for (final listWidth in _smallListWidths) {
    for (final count in _itemCounts) {
      testWidgets('Small group of $count tiles in a list $listWidth wide '
          'stays where the Wrap put them', (tester) async {
        _bigView(tester);
        await tester.pumpWidget(
          _host(
            SizedBox(
              width: listWidth,
              child: TurntileGroup<int>(
                groupIndex: 0,
                groupTitle: 'Group',
                items: List<int>.generate(count, (i) => i),
                itemBuilder: (context, item) => const SizedBox.expand(),
                gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
              ),
            ),
          ),
        );

        // The group pads its content by 16 on both sides.
        final width = listWidth - 32;
        const gap = 4.0;
        final columns = max((width / (88 + gap)).floor(), 1);
        final rows = (count / columns).ceil();
        final cell = (width - columns * gap) / columns;

        final grid = find.byType(TileGrid);
        final managed = find.byType(ManagedTurntileScreenItem);
        expect(managed, findsNWidgets(count));

        final origin = tester.getTopLeft(grid);
        for (var i = 0; i < count; i++) {
          final row = i ~/ columns;
          final column = i % columns;
          final rect = tester.getRect(managed.at(i)).shift(-origin);
          final expected = Rect.fromLTWH(
            column * (cell + gap),
            row * (cell + gap),
            cell,
            cell,
          );
          expect(rect.left, closeTo(expected.left, _tolerance));
          expect(rect.top, closeTo(expected.top, _tolerance));
          expect(rect.width, closeTo(expected.width, _tolerance));
          expect(rect.height, closeTo(expected.height, _tolerance));
          final widget = tester.widget<ManagedTurntileScreenItem>(
            managed.at(i),
          );
          expect(widget.row, row, reason: 'row of tile $i');
          expect(widget.column, column, reason: 'column of tile $i');
        }

        final size = tester.getSize(grid);
        expect(size.width, width.floorToDouble());
        expect(size.height, (cell * rows + gap * (rows - 1)).ceilToDouble());
      });
    }
  }
}
