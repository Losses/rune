import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_display.dart';
import 'package:rune/widgets/visualizer/lab/lab_filter.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

const _columns = [
  labSectionsColumn,
  labMarksColumn,
  LabColumn(
    id: 'shot',
    name: 'Shot',
    group: 'Director',
    kind: LabColumnKind.category,
  ),
  LabColumn(
    id: 'drive',
    name: 'Drive',
    group: 'Signals',
    kind: LabColumnKind.number,
  ),
  LabColumn(
    id: 'low',
    name: 'Low',
    group: 'Signals',
    kind: LabColumnKind.number,
  ),
  LabColumn(
    id: 'input:glow',
    name: 'Glow',
    group: 'Inputs',
    kind: LabColumnKind.number,
    binds: ['drive'],
  ),
];

List<String> _ids(LabRowsLayout layout) => [
  for (final r in layout.rows)
    r.type == LabRowType.group ? '#${r.groupName}' : r.column!.id,
];

LabSeriesCatalogue _catalogue() => const LabSeriesCatalogue(
  version: 1,
  rate: 20,
  columns: [
    LabSeriesColumn(
      id: 'num',
      name: 'Num',
      group: VisualizerLabColumnGroup.signals,
      kind: VisualizerLabColumnKind.number,
    ),
    LabSeriesColumn(
      id: 'phase',
      name: 'Phase',
      group: VisualizerLabColumnGroup.signals,
      kind: VisualizerLabColumnKind.phase,
    ),
    LabSeriesColumn(
      id: 'event',
      name: 'Event',
      group: VisualizerLabColumnGroup.signals,
      kind: VisualizerLabColumnKind.event,
    ),
    LabSeriesColumn(
      id: 'flag',
      name: 'Flag',
      group: VisualizerLabColumnGroup.signals,
      kind: VisualizerLabColumnKind.bool,
    ),
    LabSeriesColumn(
      id: 'cat',
      name: 'Cat',
      group: VisualizerLabColumnGroup.signals,
      kind: VisualizerLabColumnKind.category,
    ),
    LabSeriesColumn(
      id: 'input:glow',
      name: 'Glow',
      group: VisualizerLabColumnGroup.inputs,
      kind: VisualizerLabColumnKind.number,
    ),
  ],
);

// A sample of the catalogue's six columns.
LabSeriesSample _sample(int bucket, List<double> values) =>
    LabSeriesSample(bucket: bucket, columnsVersion: 1, values: values);

const _nan = double.nan;

// Records 200 buckets, 10 s, with [at] giving the values of a bucket.
void _record(LabSeriesStore store, List<double> Function(int) at) {
  store.addSamples([for (var b = 0; b < 200; b++) _sample(b, at(b))]);
}

void main() {
  group('pinned rows in the layout', () {
    test('the pinned rows come first, in pin order, and once', () {
      final selection = LabRowSelection.all(
        _columns,
      ).unpin('sections').unpin('marks').pin('low').pin('shot');
      final layout = LabRowsLayout.build(_columns, selection);
      expect(_ids(layout), [
        '#Pinned',
        'low',
        'shot',
        '#Plan & Actual',
        'sections',
        'marks',
        '#Signals',
        'drive',
        '#Inputs',
        'input:glow',
      ]);
      expect(layout.pinnedCount, 3);
      expect(layout.rows[3].y, layout.pinnedEnd);
    });

    test('a pinned row that is not shown is not drawn', () {
      final selection = LabRowSelection.all(
        _columns,
      ).withVisible(_columns[3], on: false).pin('drive');
      final layout = LabRowsLayout.build(_columns, selection);
      expect(_ids(layout), isNot(contains('drive')));
      expect(_ids(layout).first, '#Pinned');
      expect(layout.pinnedCount, 3);
    });

    test('without pins there is no pinned block', () {
      final selection = LabRowSelection.all(
        _columns,
      ).unpin('sections').unpin('marks');
      final layout = LabRowsLayout.build(_columns, selection);
      expect(layout.pinnedCount, 0);
      expect(layout.fixed(500), (count: 0, height: 0.0));
      expect(_ids(layout).first, '#Plan & Actual');
    });

    test('the fixed rows take at most 40% of the rows and whole rows', () {
      final layout = LabRowsLayout.build(
        _columns,
        LabRowSelection.all(_columns),
      );
      // The heading, the sections and the marks, then the first scrolling row.
      expect(layout.pinnedCount, 3);
      final all = layout.fixed(400);
      expect(all.count, 3);
      expect(all.height, layout.pinnedEnd);
      expect(all.height, lessThanOrEqualTo(400 * 0.4));
      // Less room: the marks go to the scrolling part.
      final some = layout.fixed(200);
      expect(some.count, 2);
      expect(some.height, layout.rows[2].y);
      expect(some.height, lessThanOrEqualTo(200 * 0.4));
      // Not even the first row fits: nothing is fixed.
      expect(layout.fixed(100), (count: 0, height: 0.0));
    });

    test('hit testing follows the fixed rows and the scroll', () {
      final layout = LabRowsLayout.build(
        _columns,
        LabRowSelection.all(_columns),
      );
      const rowsHeight = 400.0;
      final f = layout.fixed(rowsHeight);
      final sections = layout.rows[1];
      final drive = layout.rows.firstWhere((r) => r.column?.id == 'drive');
      // A fixed row does not move with the scroll.
      expect(
        layout.rowAtView(sections.y + 2, 40, rowsHeight)!.column!.id,
        'sections',
      );
      // Below them, the rows are as far down as the scroll says.
      final viewY = drive.y + 2 - 40;
      expect(viewY, greaterThan(f.height));
      expect(layout.rowAtView(viewY, 40, rowsHeight)!.column!.id, 'drive');
      expect(layout.contentY(viewY, 40, rowsHeight), drive.y + 2);
      expect(layout.contentY(sections.y, 40, rowsHeight), sections.y);
      // The scrolled rows are not met under the fixed ones.
      expect(layout.rowAtView(f.height - 3, 40, rowsHeight), isNot(drive));
    });

    test('rows with nothing are left out, the pinned ones excepted', () {
      final selection = LabRowSelection.all(_columns).pin('low');
      const empty = {'drive', 'low', 'sections'};
      final layout = LabRowsLayout.build(_columns, selection, empty: empty);
      final ids = _ids(layout);
      expect(ids, isNot(contains('drive')));
      expect(ids, contains('low'));
      expect(ids, contains('sections'));
      // Left alone when the selection does not hide them.
      final shown = LabRowsLayout.build(
        _columns,
        selection,
        display: const LabDisplay(hideEmpty: false),
        empty: empty,
      );
      expect(_ids(shown), contains('drive'));
    });
  });

  group('rows with nothing in the song', () {
    test('nothing is empty before the plan or ten seconds', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      store.addSamples([
        for (var b = 0; b < 199; b++) _sample(b, List.filled(6, _nan)),
      ]);
      expect(store.emptyColumns, isEmpty);
      store.addSamples([_sample(199, List.filled(6, _nan))]);
      expect(store.emptyColumns, {
        'num',
        'phase',
        'event',
        'flag',
        'cat',
        'input:glow',
      });
    });

    test('a seek far into the song does not count as ten seconds', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      store.addSamples([_sample(2400, List.filled(6, _nan))]);
      expect(store.emptyColumns, isEmpty);
    });

    test('the set stays the same while what is in it does', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      _record(store, (_) => List.filled(6, _nan));
      final first = store.emptyColumns;
      store.addSamples([_sample(200, List.filled(6, _nan))]);
      expect(identical(store.emptyColumns, first), isTrue);
      store.addSamples([
        _sample(201, [1, _nan, _nan, _nan, _nan, _nan]),
        _sample(202, [2, _nan, _nan, _nan, _nan, _nan]),
      ]);
      expect(identical(store.emptyColumns, first), isFalse);
      expect(store.emptyColumns, isNot(contains('num')));
    });

    test('each kind has its own rule', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      _record(store, (b) {
        return [
          // A number that stays at the bottom of its range.
          0,
          // A phase that does.
          (b % 20) / 20,
          // Events that never happen.
          0,
          // A bool that is on once.
          b == 7 ? 1 : 0,
          // A category with values.
          1,
          // An input that is a constant.
          0.25,
        ];
      });
      expect(store.emptyColumns, {'num', 'event', 'input:glow'});
    });

    test('a number that holds away from the bottom of its range is shown', () {
      final store = LabSeriesStore()
        ..setCatalogue(
          const LabSeriesCatalogue(
            version: 1,
            rate: 20,
            columns: [
              LabSeriesColumn(
                id: 'tempo',
                name: 'tempo',
                group: VisualizerLabColumnGroup.signals,
                kind: VisualizerLabColumnKind.number,
                lo: 60,
                hi: 240,
              ),
              LabSeriesColumn(
                id: 'drive',
                name: 'drive',
                group: VisualizerLabColumnGroup.signals,
                kind: VisualizerLabColumnKind.number,
              ),
            ],
          ),
        );
      store.addSamples([
        for (var b = 0; b < 200; b++)
          LabSeriesSample(bucket: b, columnsVersion: 1, values: [128, 0]),
      ]);
      expect(store.emptyColumns, {'drive'});
    });

    test('a value that comes later takes a row off the set', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      _record(store, (b) => [0, 0, 0, 0, _nan, 0]);
      final before = store.emptyColumns;
      expect(before, hasLength(6));
      // The same set is given until something changes.
      expect(identical(store.emptyColumns, before), isTrue);
      store.addSamples([
        _sample(10, [0, 0, 2, 0, 1, 0]),
      ]);
      expect(store.emptyColumns, {'num', 'phase', 'flag', 'input:glow'});
    });

    test('the plan opens the gate and counts as something to show', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      expect(store.emptyColumns, isEmpty);
      // Six columns of four buckets; only the first changes.
      final values = [
        for (var c = 0; c < 6; c++)
          for (var b = 0; b < 4; b++) c == 0 ? b / 4 : 0.0,
      ];
      expect(
        store.importPlan(
          LabSeriesPlan(
            columnsVersion: 1,
            rate: 20,
            buckets: 4,
            values: values,
          ),
        ),
        isTrue,
      );
      // A category with values has something to show, whatever they are.
      expect(store.emptyColumns, {'phase', 'event', 'flag', 'input:glow'});
      store.clearPlan();
      expect(store.emptyColumns, isEmpty);
    });

    test('a new song starts afresh', () {
      final store = LabSeriesStore()..setCatalogue(_catalogue());
      _record(store, (b) => List.filled(6, 0.0));
      expect(store.emptyColumns, isNotEmpty);
      store.reset();
      expect(store.emptyColumns, isEmpty);
      store.setCatalogue(_catalogue());
      expect(store.emptyColumns, isEmpty);
    });
  });

  group('row menu and the Rows dialog', () {
    Future<LabRowsModel> pump(
      WidgetTester tester, {
      Size size = const Size(1200, 700),
      Set<String> empty = const {},
      ValueChanged<double>? onSeek,
    }) async {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final model = LabRowsModel()..setSelection(LabRowSelection.all(_columns));
      await tester.pumpWidget(
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
          child: FluentApp(
            // As the app's root: the reveal of its items needs it.
            builder: (context, child) => RevealEffectContext(child: child!),
            home: ScaffoldPage(
              padding: EdgeInsets.zero,
              content: LabTimeline(
                data: LabTimelineData(
                  length: 60,
                  columns: _columns,
                  emptyColumns: empty,
                ),
                rows: model,
                onSeek: onSeek ?? (_) {},
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      return model;
    }

    // The point on the name of [id]'s row.
    Offset nameOf(WidgetTester tester, LabRowSelection selection, String id) {
      final layout = LabRowsLayout.build(_columns, selection);
      final row = layout.rows.firstWhere((r) => r.column?.id == id);
      final rect = tester.getRect(find.byType(CustomPaint).last);
      return Offset(
        rect.left + 40,
        rect.top + LabGeometry.ruler + row.y + row.height / 2,
      );
    }

    testWidgets('Hide Row and Pin to Top on a right click', (tester) async {
      final seeks = <double>[];
      final model = await pump(tester, onSeek: seeks.add);
      final before = model.selection;

      await tester.tapAt(
        nameOf(tester, before, 'drive'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(find.text('Hide Row'), findsOneWidget);
      expect(find.text('Pin to Top'), findsOneWidget);
      expect(find.text('Show Related Rows'), findsOneWidget);
      expect(find.text('Show in Rows…'), findsOneWidget);
      expect(seeks, isEmpty);

      await tester.tap(find.text('Pin to Top'));
      await tester.pumpAndSettle();
      expect(model.selection.pins, ['sections', 'marks', 'drive']);
      expect(find.text('Hide Row'), findsNothing);

      // The pinned row is now fixed under the ruler, and has Unpin.
      await tester.tapAt(
        nameOf(tester, model.selection, 'drive'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      expect(find.text('Unpin'), findsOneWidget);
      await tester.tap(find.text('Hide Row'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_columns[3]), isFalse);
      expect(seeks, isEmpty);
    });

    testWidgets('the strip items and Show Related Rows change the rows', (
      tester,
    ) async {
      final model = await pump(tester);
      await tester.tapAt(
        nameOf(tester, model.selection, 'drive'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Plan Only'));
      await tester.pumpAndSettle();
      expect(model.selection.planOn('drive'), isTrue);
      expect(model.selection.actualOn('drive'), isFalse);

      await tester.tapAt(
        nameOf(tester, model.selection, 'drive'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Show Related Rows'));
      await tester.pumpAndSettle();
      expect(model.selection.isShown(_columns[5]), isTrue);
    });

    testWidgets('a click with Control opens the menu and does not seek', (
      tester,
    ) async {
      final seeks = <double>[];
      final model = await pump(tester, onSeek: seeks.add);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.tapAt(
        nameOf(tester, model.selection, 'shot'),
        kind: PointerDeviceKind.mouse,
      );
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
      expect(find.text('Hide Row'), findsOneWidget);
      expect(seeks, isEmpty);
    });

    testWidgets('Show in Rows opens the dialog at the row, marked', (
      tester,
    ) async {
      final model = await pump(tester);
      await tester.tapAt(
        nameOf(tester, model.selection, 'drive'),
        buttons: kSecondaryButton,
        kind: PointerDeviceKind.mouse,
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Show in Rows…'));
      await tester.pumpAndSettle();
      expect(find.byType(LabRowsDialog), findsOneWidget);
      ListTile tileOf(String name) => tester.widget<ListTile>(
        find
            .ancestor(of: find.text(name).last, matching: find.byType(ListTile))
            .first,
      );
      expect(tileOf('Drive').tileColor, isNotNull);
      expect(tileOf('Low').tileColor, isNull);
      // On screen, in the dialog's list.
      final list = tester.getRect(
        find.descendant(
          of: find.byType(LabRowsDialog),
          matching: find.byType(CustomScrollView),
        ),
      );
      expect(list.contains(tester.getCenter(find.text('Drive').last)), isTrue);
    });
  });
}
