import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/widgets/start_screen/managed_start_screen_item.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/start_screen/start_group.dart';
import 'package:rune/widgets/tile_group/tile_grid.dart';
import 'package:rune/widgets/tile_group/tile_layout.dart';

// A group that lays out again, for example when the window gets lower, moves
// its tiles. It must not rebuild them from scratch: their State stays.

class _Probe extends StatefulWidget {
  const _Probe({super.key});

  @override
  State<_Probe> createState() => _ProbeState();
}

int _inits = 0;
int _disposes = 0;

class _ProbeState extends State<_Probe> {
  @override
  void initState() {
    super.initState();
    _inits++;
  }

  @override
  void dispose() {
    _disposes++;
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => const SizedBox.expand();
}

Widget _host(StartScreenLayoutManager manager, Widget child) {
  return FluentTheme(
    data: FluentThemeData(),
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: manager,
        child: Align(alignment: Alignment.topLeft, child: child),
      ),
    ),
  );
}

Offset _relativeTopLeft(WidgetTester tester, Finder finder, Finder grid) {
  return tester.getTopLeft(finder) - tester.getTopLeft(grid);
}

void main() {
  const gap = 4.0;
  const cell = 120.0;
  const step = cell + gap;

  testWidgets('TileGrid keeps the State of its tiles when the layout changes', (
    tester,
  ) async {
    _inits = 0;
    _disposes = 0;
    const spans = [
      TileSpan(1, 1),
      TileSpan(1, 1),
      TileSpan(1, 1),
      TileSpan(1, 1),
      TileSpan(1, 1),
      TileSpan(1, 1),
    ];

    Widget grid(int rows) {
      final layout = layoutTiles(spans, rows: rows);
      return TileGrid(
        layout: layout,
        spans: spans,
        cellSize: cell,
        gapSize: gap,
        size: TileGrid.packedSize(layout, cell, gap),
        children: [for (var i = 0; i < spans.length; i++) const _Probe()],
      );
    }

    final manager = StartScreenLayoutManager();
    await tester.pumpWidget(_host(manager, grid(3)));
    final probes = find.byType(_Probe);
    expect(probes, findsNWidgets(spans.length));
    final before = [
      for (var i = 0; i < spans.length; i++)
        tester.state<_ProbeState>(probes.at(i)),
    ];
    final gridFinder = find.byType(TileGrid);
    // Column by column, three rows: tile 3 is the top of the second column.
    expect(
      _relativeTopLeft(tester, probes.at(3), gridFinder),
      const Offset(step, 0),
    );
    expect(_inits, spans.length);

    await tester.pumpWidget(_host(manager, grid(2)));

    // Two rows: tile 3 is the bottom of the second column.
    expect(
      _relativeTopLeft(tester, probes.at(3), gridFinder),
      const Offset(step, step),
    );
    expect(_inits, spans.length, reason: 'no tile was built again');
    expect(_disposes, 0, reason: 'no tile was disposed');
    for (var i = 0; i < spans.length; i++) {
      expect(
        tester.state<_ProbeState>(probes.at(i)),
        same(before[i]),
        reason: 'State of tile $i',
      );
    }
  });

  testWidgets('TileGrid keeps the State of keyed tiles that swap places', (
    tester,
  ) async {
    const spans = [TileSpan(1, 1), TileSpan(1, 1)];
    final layout = layoutTiles(spans, rows: 1);
    // Local keys: a GlobalKey would keep the State no matter how the Stack
    // matches its children.
    const a = ValueKey('a');
    const b = ValueKey('b');

    Widget grid(List<Key> keys) {
      return TileGrid(
        layout: layout,
        spans: spans,
        cellSize: cell,
        gapSize: gap,
        size: TileGrid.packedSize(layout, cell, gap),
        children: [for (final k in keys) _Probe(key: k)],
      );
    }

    await tester.pumpWidget(_host(StartScreenLayoutManager(), grid([a, b])));
    final stateA = tester.state<_ProbeState>(find.byKey(a));
    final stateB = tester.state<_ProbeState>(find.byKey(b));
    final gridFinder = find.byType(TileGrid);
    expect(_relativeTopLeft(tester, find.byKey(a), gridFinder), Offset.zero);

    await tester.pumpWidget(_host(StartScreenLayoutManager(), grid([b, a])));

    expect(tester.state<_ProbeState>(find.byKey(a)), same(stateA));
    expect(tester.state<_ProbeState>(find.byKey(b)), same(stateB));
    expect(
      _relativeTopLeft(tester, find.byKey(b), gridFinder),
      Offset.zero,
      reason: 'b is now first',
    );
    expect(
      _relativeTopLeft(tester, find.byKey(a), gridFinder),
      const Offset(step, 0),
    );
  });

  testWidgets('StartGroup keeps the State of its tiles when the rows change', (
    tester,
  ) async {
    final manager = StartScreenLayoutManager();
    final items = List<int>.generate(6, (i) => i);

    Widget group(double maxHeight) {
      return StartGroup<int>(
        groupIndex: 0,
        groupTitle: 'Group',
        items: items,
        itemBuilder: (context, item) => const SizedBox.expand(),
        constraints: BoxConstraints(maxWidth: 2000, maxHeight: maxHeight),
        gapSize: gap,
        direction: Axis.vertical,
      );
    }

    await tester.pumpWidget(_host(manager, group(396)));
    final managed = find.byType(ManagedStartScreenItem);
    final grid = find.byType(TileGrid);
    expect(managed, findsNWidgets(items.length));
    final before = [
      for (var i = 0; i < items.length; i++)
        tester.state<ManagedStartScreenItemState>(managed.at(i)),
    ];
    // Three rows, column by column.
    for (var i = 0; i < items.length; i++) {
      expect(
        _relativeTopLeft(tester, managed.at(i), grid),
        Offset((i ~/ 3) * step, (i % 3) * step),
        reason: 'tile $i with three rows',
      );
    }

    await tester.pumpWidget(_host(manager, group(272)));

    // Two rows, column by column.
    for (var i = 0; i < items.length; i++) {
      expect(
        _relativeTopLeft(tester, managed.at(i), grid),
        Offset((i ~/ 2) * step, (i % 2) * step),
        reason: 'tile $i with two rows',
      );
    }
    expect(managed, findsNWidgets(items.length));
    for (var i = 0; i < items.length; i++) {
      expect(
        tester.state<ManagedStartScreenItemState>(managed.at(i)),
        same(before[i]),
        reason: 'State of tile $i',
      );
    }
  });
}
