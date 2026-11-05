import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_display.dart';
import 'package:rune/widgets/visualizer/lab/lab_draw_style.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

final _columns = [
  labSectionsColumn,
  labMarksColumn,
  const LabColumn(
    id: 'tempo',
    name: 'Tempo',
    group: 'Signals',
    kind: LabColumnKind.number,
    lo: 60,
    hi: 240,
  ),
  const LabColumn(
    id: 'hits',
    name: 'Hits',
    group: 'Signals',
    kind: LabColumnKind.event,
  ),
  const LabColumn(
    id: 'shot',
    name: 'Shot',
    group: 'Director',
    kind: LabColumnKind.category,
  ),
  const LabColumn(
    id: 'drive',
    name: 'Drive',
    group: 'Signals',
    kind: LabColumnKind.number,
  ),
  const LabColumn(
    id: 'band_low',
    name: 'Band Low',
    group: 'Signals',
    kind: LabColumnKind.number,
    plan: LabPlanAvailability.no,
  ),
];

LabColumn _col(String id) => _columns.firstWhere((c) => c.id == id);

LabRow _row(LabRowsLayout layout, String id) =>
    layout.rows.firstWhere((r) => r.column?.id == id);

void main() {
  final all = LabRowSelection.all(_columns);

  test('no row moves when the strips or the lines change', () {
    List<(String?, double, double)> places(
      LabRowSelection selection,
      LabDisplay display,
    ) => [
      for (final r in LabRowsLayout.build(
        _columns,
        selection,
        display: display,
      ).rows)
        (r.column?.id ?? r.groupName, r.y, r.height),
    ];

    const both = LabDisplay();
    final before = places(all, both);
    for (final display in [
      both.withStripMode(LabStripMode.plan),
      both.withStripMode(LabStripMode.actual),
      both.withAllLines(true),
      both.withExpanded('drive', on: true),
    ]) {
      expect(places(all, display), before, reason: '$display');
    }
    // Nor when a row shows one of its strips.
    for (final id in ['marks', 'shot', 'tempo', 'hits', 'drive']) {
      expect(
        places(all.setPart(id, plan: false, actual: true), both),
        before,
        reason: id,
      );
    }
    // The strips that are shown share the row.
    final plan = LabRowsLayout.build(
      _columns,
      all,
      display: both.withStripMode(LabStripMode.plan),
    );
    for (final id in ['marks', 'shot', 'tempo', 'hits']) {
      final r = _row(plan, id);
      expect(r.partCount, 1, reason: id);
      expect(r.stripHeight, r.height, reason: id);
    }
  });

  group('display rules', () {
    test('both draws what each row has on', () {
      const d = LabDisplay();
      expect(d.stripMode, LabStripMode.both);
      expect(d.showsPlan(all, _col('shot')), isTrue);
      expect(d.showsActual(all, _col('shot')), isTrue);
      final only = all.setPart('shot', plan: false, actual: true);
      expect(d.showsPlan(only, _col('shot')), isFalse);
      expect(d.showsActual(only, _col('shot')), isTrue);
    });

    test('the plan mode draws the plan, else the actual', () {
      final d = const LabDisplay().withStripMode(LabStripMode.plan);
      expect(d.showsPlan(all, _col('shot')), isTrue);
      expect(d.showsActual(all, _col('shot')), isFalse);
      // Only an actual strip: it keeps it.
      expect(d.showsPlan(all, _col('band_low')), isFalse);
      expect(d.showsActual(all, _col('band_low')), isTrue);
      // A row with only its actual on still draws its plan.
      final only = all.setPart('shot', plan: false, actual: true);
      expect(d.showsPlan(only, _col('shot')), isTrue);
      expect(d.showsActual(only, _col('shot')), isFalse);
    });

    test('the actual mode draws the actual, else the plan', () {
      final d = const LabDisplay().withStripMode(LabStripMode.actual);
      expect(d.showsActual(all, _col('shot')), isTrue);
      expect(d.showsPlan(all, _col('shot')), isFalse);
      expect(d.showsPlan(all, _col('sections')), isTrue);
      expect(d.showsActual(all, _col('sections')), isFalse);
      expect(d.showsActual(all, _col('band_low')), isTrue);
    });

    test('a row that is hidden stays hidden in every mode', () {
      final hidden = all.withVisible(_col('shot'), on: false);
      for (final mode in LabStripMode.values) {
        final d = const LabDisplay().withStripMode(mode);
        expect(d.showsPlan(hidden, _col('shot')), isFalse, reason: '$mode');
        expect(d.showsActual(hidden, _col('shot')), isFalse, reason: '$mode');
      }
    });

    test('the style follows the lines and the expanded levels', () {
      final drive = _col('drive');
      const d = LabDisplay();
      expect(d.styleOf(drive), LabDrawStyle.band);
      expect(d.withAllLines(true).styleOf(drive), LabDrawStyle.line);
      expect(d.withExpanded('drive', on: true).isExpanded('drive'), isTrue);
      expect(d.withAllLines(true).styleOf(_col('shot')), LabDrawStyle.blocks);
      expect(
        d.withExpanded('drive', on: true).withAllLines(true).expanded,
        isEmpty,
      );
    });
  });

  group('display JSON', () {
    test('it survives the JSON', () {
      final d = const LabDisplay()
          .withStripMode(LabStripMode.actual)
          .withAllLines(true)
          .withExpanded('drive', on: true);
      expect(LabDisplay.fromJson(d.toJson()), d);
      expect(d.toMap(), {
        'strips': 'actual',
        'lines': true,
        'expanded': ['drive'],
        'hideEmpty': true,
      });
    });

    test('hideEmpty is saved, and missing means on', () {
      const d = LabDisplay(hideEmpty: false);
      expect(d.toMap()['hideEmpty'], isFalse);
      expect(LabDisplay.fromJson(d.toJson()), d);
      expect(LabDisplay.fromJson('{"strips":"plan"}').hideEmpty, isTrue);
    });

    test('missing or unknown values give the defaults', () {
      expect(LabDisplay.fromJson(null), const LabDisplay());
      expect(LabDisplay.fromJson('nope'), const LabDisplay());
      expect(LabDisplay.fromJson('[1]'), const LabDisplay());
      expect(LabDisplay.fromJson('{}'), const LabDisplay());
      expect(
        LabDisplay.fromJson('{"strips":"x","lines":3,"expanded":[1,"a"]}'),
        const LabDisplay().withExpanded('a', on: true),
      );
    });

    test('equal displays are equal', () {
      const a = LabDisplay();
      final b = a.withStripMode(LabStripMode.plan);
      expect(a == b, isFalse);
      expect(b, const LabDisplay().withStripMode(LabStripMode.plan));
      expect(
        b.hashCode,
        const LabDisplay().withStripMode(LabStripMode.plan).hashCode,
      );
      expect(a.withAllLines(true) == a, isFalse);
      expect(a.withHideEmpty(false) == a, isFalse);
      expect(a.withHideEmpty(false).hashCode == a.hashCode, isFalse);
      expect(identical(a.withHideEmpty(true), a), isTrue);
      expect(a.withExpanded('x', on: true) == a, isFalse);
      expect(identical(a.withStripMode(LabStripMode.both), a), isTrue);
    });
  });

  group('layout', () {
    test('it honours hideEmpty of the display', () {
      const empty = {'drive'};
      bool has(LabDisplay d) => LabRowsLayout.build(
        _columns,
        all,
        display: d,
        empty: empty,
      ).rows.any((r) => r.column?.id == 'drive');
      expect(has(const LabDisplay()), isFalse);
      expect(has(const LabDisplay(hideEmpty: false)), isTrue);
    });

    test('it honours the display', () {
      final plan = const LabDisplay().withStripMode(LabStripMode.plan);
      final layout = LabRowsLayout.build(_columns, all, display: plan);
      final shot = _row(layout, 'shot');
      expect(shot.showPlan, isTrue);
      expect(shot.showActual, isFalse);
      expect(_row(layout, 'band_low').showActual, isTrue);

      final lines = const LabDisplay().withAllLines(true);
      expect(
        _row(LabRowsLayout.build(_columns, all, display: lines), 'drive').style,
        LabDrawStyle.line,
      );
      expect(
        _row(LabRowsLayout.build(_columns, all), 'drive').style,
        LabDrawStyle.band,
      );
    });
  });

  group('Display flyout', () {
    Future<LabRowsModel> open(
      WidgetTester tester, {
      Set<String> empty = const {},
      LabRowSelection? selection,
    }) async {
      tester.view.physicalSize = const Size(1200, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final model = LabRowsModel();
      if (selection != null) model.setSelection(selection);
      await tester.pumpWidget(
        FluentApp(
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
              onSeek: (_) {},
            ),
          ),
        ),
      );
      await tester.pump();
      await tester.tap(find.byIcon(Symbols.tune));
      await tester.pumpAndSettle();
      return model;
    }

    testWidgets('sits before the Rows button and opens a flyout', (
      tester,
    ) async {
      await open(tester);
      expect(find.byIcon(Symbols.tune), findsOneWidget);
      expect(find.text('Strips'), findsOneWidget);
      expect(find.text('Draw Levels as Lines'), findsOneWidget);
    });

    testWidgets('its labels are the controls\' own, as the app\'s dialogs '
        'have them', (tester) async {
      await open(tester);
      expect(
        tester.widget<InfoLabel>(find.byType(InfoLabel)).label.toPlainText(),
        'Strips',
      );
      for (final label in [
        'Draw Levels as Lines',
        'Hide Rows with Nothing in This Song',
      ]) {
        expect(
          find.descendant(
            of: find.byType(ToggleSwitch),
            matching: find.text(label),
          ),
          findsOneWidget,
          reason: label,
        );
      }
    });

    testWidgets('Plan, Actual and Both set the strips at once', (tester) async {
      final model = await open(tester);
      await tester.tap(find.text('Plan'));
      await tester.pumpAndSettle();
      expect(model.display.stripMode, LabStripMode.plan);
      await tester.tap(find.text('Actual'));
      await tester.pumpAndSettle();
      expect(model.display.stripMode, LabStripMode.actual);
      await tester.tap(find.text('Both'));
      await tester.pumpAndSettle();
      expect(model.display.stripMode, LabStripMode.both);
      // The rows and the view are not touched.
      expect(model.selection, LabRowSelection.defaults);
      expect(model.isEdited(_columns), isFalse);
    });

    testWidgets('Draw Levels as Lines switches every level', (tester) async {
      final model = await open(tester);
      expect(model.display.allLines, isFalse);
      await tester.tap(find.text('Draw Levels as Lines'));
      await tester.pumpAndSettle();
      expect(model.display.allLines, isTrue);
      await tester.tap(find.text('Draw Levels as Lines'));
      await tester.pumpAndSettle();
      expect(model.display.allLines, isFalse);
      expect(model.isEdited(_columns), isFalse);
    });

    testWidgets('Hide Rows with Nothing in This Song switches at once', (
      tester,
    ) async {
      final model = await open(tester);
      expect(model.display.hideEmpty, isTrue);
      await tester.tap(find.text('Hide Rows with Nothing in This Song'));
      await tester.pumpAndSettle();
      expect(model.display.hideEmpty, isFalse);
      expect(model.isEdited(_columns), isFalse);
      await tester.tap(find.text('Hide Rows with Nothing in This Song'));
      await tester.pumpAndSettle();
      expect(model.display.hideEmpty, isTrue);
    });

    testWidgets('the count leaves out the pinned rows, and goes when off', (
      tester,
    ) async {
      final model = await open(
        tester,
        empty: {'drive', 'band_low', 'marks'},
        selection: all,
      );
      // The marks are pinned by default.
      expect(find.text('Hide Rows with Nothing in This Song (2)'), findsOne);
      await tester.tap(find.textContaining('Hide Rows with Nothing'));
      await tester.pumpAndSettle();
      expect(model.display.hideEmpty, isFalse);
      expect(find.text('Hide Rows with Nothing in This Song'), findsOne);
    });
  });
}
