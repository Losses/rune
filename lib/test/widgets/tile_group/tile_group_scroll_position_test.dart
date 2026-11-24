import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/start_screen/start_group.dart';
import 'package:rune/widgets/tile_group/tile_layout.dart';
import 'package:rune/widgets/turntile/turntile_group.dart';

// Scrolling to a group adds up the sizes of the groups before it. That sum
// must equal where the groups really are laid out.

const _tolerance = 1e-6;

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

void main() {
  testWidgets('Large groups start where the summed widths say', (tester) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(30000, 2000);
    addTearDown(tester.view.reset);

    const counts = [1, 3, 7, 12, 2, 30];
    // The group with one tile has a title wider than its tile.
    const titles = [
      'A title that is much wider than a single tile',
      'A',
      'A',
      'A',
      'A',
      'A',
    ];
    final groups = [
      for (var i = 0; i < counts.length; i++)
        StartGroup<int>(
          groupIndex: i,
          groupTitle: titles[i],
          items: List<int>.generate(counts[i], (j) => j),
          itemBuilder: (context, item) => const SizedBox.expand(),
          constraints: const BoxConstraints(maxWidth: 30000, maxHeight: 396),
          gapSize: 4,
          direction: Axis.vertical,
        ),
    ];
    const rowKey = ValueKey('row');
    late BuildContext context;

    await tester.pumpWidget(
      _host(
        Builder(
          builder: (c) {
            context = c;
            return SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                key: rowKey,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: groups,
              ),
            );
          },
        ),
      ),
    );

    final rowLeft = tester.getTopLeft(find.byKey(rowKey)).dx;
    final laidOut = find.byType(StartGroup<int>);
    expect(laidOut, findsNWidgets(groups.length));
    var sum = 0.0;
    for (var n = 0; n < groups.length; n++) {
      expect(
        tester.getTopLeft(laidOut.at(n)).dx - rowLeft,
        closeTo(sum, _tolerance),
        reason: 'left edge of group $n',
      );
      // The context comes from the host built above, it is still mounted.
      // ignore: use_build_context_synchronously
      sum += groups[n].widthIn(context);
    }
    expect(
      tester.getSize(find.byKey(rowKey)).width,
      closeTo(sum, _tolerance),
      reason: 'width of the whole row',
    );
  });

  testWidgets('Small groups start where the summed heights say', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(2000, 30000);
    addTearDown(tester.view.reset);

    const counts = [1, 5, 12, 3, 30];
    const titles = [
      'A',
      'A title long enough that it wraps onto several lines in a narrow list',
      'A',
      'A',
      'A',
    ];
    final groups = [
      for (var i = 0; i < counts.length; i++)
        TurntileGroup<int>(
          groupIndex: i,
          groupTitle: titles[i],
          items: List<int>.generate(counts[i], (j) => j),
          itemBuilder: (context, item) => const SizedBox.expand(),
          gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
          tileSpanBuilder: i == 2
              ? (item) =>
                    item == 0 ? const TileSpan(2, 2) : const TileSpan(1, 1)
              : null,
        ),
    ];
    const columnKey = ValueKey('column');
    late BuildContext context;

    await tester.pumpWidget(
      _host(
        Builder(
          builder: (c) {
            context = c;
            return SizedBox(
              width: 400,
              child: Column(
                key: columnKey,
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: groups,
              ),
            );
          },
        ),
      ),
    );

    final columnTop = tester.getTopLeft(find.byKey(columnKey)).dy;
    final laidOut = find.byType(TurntileGroup<int>);
    expect(laidOut, findsNWidgets(groups.length));
    var sum = 0.0;
    for (var n = 0; n < groups.length; n++) {
      expect(
        tester.getTopLeft(laidOut.at(n)).dy - columnTop,
        closeTo(sum, _tolerance),
        reason: 'top edge of group $n',
      );
      // ignore: use_build_context_synchronously
      sum += groups[n].heightIn(context, 400);
    }
    expect(
      tester.getSize(find.byKey(columnKey)).height,
      closeTo(sum, _tolerance),
      reason: 'height of the whole column',
    );
  });
}
