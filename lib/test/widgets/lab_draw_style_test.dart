import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_display.dart';
import 'package:rune/widgets/visualizer/lab/lab_draw_style.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_series.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline_math.dart';
import 'package:rune/widgets/visualizer/lab/lab_tooltip.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

LabColumn _number(
  String id, {
  double lo = 0,
  double hi = 1,
  double? rest,
  String? unit,
}) => LabColumn(
  id: id,
  name: id,
  group: 'Test',
  kind: LabColumnKind.number,
  lo: lo,
  hi: hi,
  rest: rest,
  unit: unit,
);

LabColumn _category(String id) => LabColumn(
  id: id,
  name: id,
  group: 'Test',
  kind: LabColumnKind.category,
  categories: const ['a', 'b', 'c'],
);

LabColumn _of(String id, LabColumnKind kind) =>
    LabColumn(id: id, name: id, group: 'Test', kind: kind);

final _columns = [
  labSectionsColumn,
  labMarksColumn,
  _number('drive'),
  _category('shot'),
  _number('age', hi: 8),
  _number('tempo', lo: 60, hi: 200, unit: 'bpm'),
  _of('hits', LabColumnKind.event),
  _of('locked', LabColumnKind.bool),
  _number('input:glow', rest: 0.5),
  _category('tier'),
];

LabColumn _col(String id) => _columns.firstWhere((c) => c.id == id);

LabRow _row(LabRowsLayout layout, String id) =>
    layout.rows.firstWhere((r) => r.column?.id == id);

void main() {
  group('default styles', () {
    test('categories are blocks, or steps for the ordered ones', () {
      expect(labDefaultStyle(_category('shot')), LabDrawStyle.blocks);
      expect(labDefaultStyle(_category('look')), LabDrawStyle.blocks);
      expect(labDefaultStyle(_category('other')), LabDrawStyle.blocks);
      for (final id in ['size', 'tier', 'pace', 'level']) {
        expect(labDefaultStyle(_category(id)), LabDrawStyle.steps, reason: id);
      }
    });

    test('numbers by what they are', () {
      for (final id in ['tempo', 'meter', 'section', 'section_label']) {
        expect(labDefaultStyle(_number(id)), LabDrawStyle.numbers, reason: id);
      }
      for (final id in ['cue_in', 'age']) {
        expect(labDefaultStyle(_number(id)), LabDrawStyle.line, reason: id);
      }
      expect(labDefaultStyle(_number('drive')), LabDrawStyle.band);
    });

    test('an input is drawn around its rest only when that is inside', () {
      expect(
        labDefaultStyle(_number('input:a', rest: 0.5)),
        LabDrawStyle.aroundRest,
      );
      expect(labDefaultStyle(_number('input:a', rest: 0)), LabDrawStyle.band);
      expect(labDefaultStyle(_number('input:a', rest: 1)), LabDrawStyle.band);
      expect(labDefaultStyle(_number('input:a')), LabDrawStyle.band);
    });

    test('the other kinds', () {
      expect(
        labDefaultStyle(_of('beat', LabColumnKind.phase)),
        LabDrawStyle.line,
      );
      expect(
        labDefaultStyle(_of('hits', LabColumnKind.event)),
        LabDrawStyle.ticks,
      );
      expect(
        labDefaultStyle(_of('locked', LabColumnKind.bool)),
        LabDrawStyle.fill,
      );
    });
  });

  group('expanding a level', () {
    final drive = _number('drive');

    test('only a level drawn as a band can be expanded', () {
      expect(labExpandable(drive), isTrue);
      expect(labExpandable(_number('input:a', rest: 0.5)), isTrue);
      for (final id in ['shot', 'age', 'tempo', 'hits', 'locked', 'tier']) {
        expect(labExpandable(_col(id)), isFalse, reason: id);
      }
      expect(labExpandable(_of('p', LabColumnKind.phase)), isFalse);
    });

    test('an expanded level is a line, folded it is its band', () {
      const none = LabDisplay();
      expect(none.styleOf(drive), LabDrawStyle.band);
      final line = none.withExpanded('drive', on: true);
      expect(line.styleOf(drive), LabDrawStyle.line);
      expect(line.withExpanded('drive', on: false), none);
      // A row that cannot be expanded keeps its own style.
      expect(
        none.withExpanded('shot', on: true).styleOf(_col('shot')),
        LabDrawStyle.blocks,
      );
    });

    test('with every level a line, a click folds just that level', () {
      const none = LabDisplay();
      final lines = none.withAllLines(true);
      expect(lines.styleOf(drive), LabDrawStyle.line);
      expect(lines.styleOf(_number('input:a', rest: 0.5)), LabDrawStyle.line);
      expect(lines.styleOf(_col('shot')), LabDrawStyle.blocks);
      expect(lines.styleOf(_col('tempo')), LabDrawStyle.numbers);
      final folded = lines.withExpanded('drive', on: true);
      expect(folded.styleOf(drive), LabDrawStyle.band);
      expect(folded.styleOf(_number('low')), LabDrawStyle.line);
      expect(folded.withExpanded('drive', on: false), lines);
      expect(lines.withAllLines(false), none);
    });

    test('the switch puts every level the same way', () {
      final one = const LabDisplay().withExpanded('drive', on: true);
      expect(one.withAllLines(true).expanded, isEmpty);
      expect(one.withAllLines(false).expanded, isEmpty);
      expect(one.withAllLines(true).allLines, isTrue);
    });
  });

  group('layout', () {
    final all = LabRowSelection.all(_columns);

    test('a line or steps row is one lane of 30 px with both parts', () {
      final layout = LabRowsLayout.build(
        _columns,
        all,
        display: const LabDisplay().withExpanded('drive', on: true),
      );
      for (final id in ['drive', 'tier']) {
        final row = _row(layout, id);
        expect(row.showPlan && row.showActual, isTrue);
        expect(row.height, 30, reason: id);
        expect(row.isLane, isTrue);
        expect(row.stripTop(0), 0);
        expect(row.stripTop(1), 0);
      }
    });

    test('blocks and numbers strips are 14 px, the others 10', () {
      final layout = LabRowsLayout.build(_columns, all);
      final shot = _row(layout, 'shot');
      expect(shot.style, LabDrawStyle.blocks);
      expect(shot.stripHeight, 14);
      expect(shot.height, 2 * 14 + 2);
      final tempo = _row(layout, 'tempo');
      expect(tempo.style, LabDrawStyle.numbers);
      expect(tempo.height, 2 * 14 + 2);
      final drive = _row(layout, 'drive');
      expect(drive.style, LabDrawStyle.band);
      expect(drive.level, isTrue);
      expect(drive.stripHeight, 14);
      expect(drive.height, 30);
      expect(_row(layout, 'hits').stripHeight, 10);
      expect(_row(layout, 'locked').stripHeight, 10);
      expect(_row(layout, 'input:glow').style, LabDrawStyle.aroundRest);
      // The actual strip is under the plan's.
      expect(shot.stripTop(1) - shot.stripTop(0), 16);
    });

    test('a level is as tall as its line, with one strip or two', () {
      const display = LabDisplay();
      for (final selection in [
        all,
        all.setPart('drive', plan: false, actual: true),
      ]) {
        final band = _row(LabRowsLayout.build(_columns, selection), 'drive');
        final line = _row(
          LabRowsLayout.build(
            _columns,
            selection,
            display: display.withExpanded('drive', on: true),
          ),
          'drive',
        );
        expect(band.style, LabDrawStyle.band);
        expect(line.style, LabDrawStyle.line);
        expect(band.height, line.height);
        // The strips share the height between them.
        final n = band.partCount;
        expect(n * band.stripHeight + (n - 1) * 2, closeTo(band.height, 1e-9));
      }
      // A row that is not a level keeps its own height.
      expect(_row(LabRowsLayout.build(_columns, all), 'hits').height, 22);
    });

    test(
      'a lane with one part is still 30 px, and a hidden row is not laid',
      () {
        final plan = all.setPart('age', plan: false, actual: true);
        final layout = LabRowsLayout.build(_columns, plan);
        final age = _row(layout, 'age');
        expect(age.style, LabDrawStyle.line);
        expect(age.partCount, 1);
        expect(age.height, 30);
        final hidden = LabRowsLayout.build(
          _columns,
          plan.withVisible(_col('age'), on: false),
        );
        expect(hidden.rows.any((r) => r.column?.id == 'age'), isFalse);
      },
    );
  });

  group('colors', () {
    final input = _number('input:a', lo: 0, hi: 2, rest: 1);

    test('around the rest: transparent at it, cool below, warm above', () {
      final at = labRestColor(input, 1)!;
      expect(at.a, lessThan(0.1));
      final below = labRestColor(input, 0)!;
      final above = labRestColor(input, 2)!;
      expect(below.a, 1);
      expect(above.a, 1);
      expect(below.b, greaterThan(below.r));
      expect(above.r, greaterThan(above.b));
      // The strength grows with the distance.
      expect(labRestColor(input, 0.5)!.a, greaterThan(at.a));
      expect(labRestColor(input, 0.5)!.a, lessThan(below.a));
      expect(labRestColor(input, double.nan), isNull);
    });

    test('the density alpha grows with the count', () {
      expect(labDensityAlpha(0), 0);
      expect(labDensityAlpha(1), greaterThan(0.25));
      expect(labDensityAlpha(2), greaterThan(labDensityAlpha(1)));
      expect(labDensityAlpha(8), greaterThan(labDensityAlpha(2)));
      expect(labDensityAlpha(1000), lessThanOrEqualTo(1));
      expect(labDensityAlpha(1000), greaterThan(0.99));
    });
  });

  group('tooltip', () {
    LabTimelineData data(List<LabColumn> columns) {
      final n = 60 * labBucketRate;
      Float32List s(double v) => Float32List.fromList(List.filled(n, v));
      return LabTimelineData(
        length: 60,
        columns: columns,
        planSeries: {'tier': s(1), 'drive': s(0.25)},
        actualSeries: {'tier': s(2), 'drive': s(0.75)},
      );
    }

    LabTip? tip(
      LabRowSelection selection,
      String id, {
      LabDisplay display = const LabDisplay(),
    }) {
      final d = data(_columns);
      final layout = LabRowsLayout.build(_columns, selection, display: display);
      final geometry = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(60),
        scrollY: 0,
        length: 60,
      );
      final row = _row(layout, id);
      return labTipAt(
        data: d,
        layout: layout,
        geometry: geometry,
        x: geometry.label + 100,
        y: LabGeometry.ruler + row.y + row.height / 2,
      );
    }

    test('a line row tells the plan and the actual, as shown', () {
      final all = LabRowSelection.all(_columns);
      final lines = const LabDisplay().withExpanded('drive', on: true);
      final both = tip(all, 'drive', display: lines)!;
      expect(both.lines.map((l) => l.key), [null, 'Plan', 'Actual']);
      expect(both.lines[1].text, '0.25');
      expect(both.lines[2].text, '0.75');
      final actual = tip(
        all.setPart('drive', plan: false, actual: true),
        'drive',
        display: lines,
      )!;
      expect(actual.lines.map((l) => l.key), [null, 'Actual']);
    });

    test('a steps row names the category', () {
      final t = tip(LabRowSelection.all(_columns), 'tier')!;
      expect(t.lines[1].text, 'b');
      expect(t.lines[2].text, 'c');
    });
  });

  group('a click on the name of a level', () {
    Future<(LabRowsModel, Rect)> pump(WidgetTester tester) async {
      tester.view.physicalSize = const Size(1200, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final model = LabRowsModel()..setSelection(LabRowSelection.all(_columns));
      await tester.pumpWidget(
        FluentApp(
          home: ScaffoldPage(
            padding: EdgeInsets.zero,
            content: LabTimeline(
              data: LabTimelineData(length: 60, columns: _columns),
              rows: model,
              onSeek: (_) {},
            ),
          ),
        ),
      );
      await tester.pump();
      return (model, tester.getRect(find.byType(CustomPaint).last));
    }

    LabDrawStyle driveStyle(LabRowsModel model) => _row(
      LabRowsLayout.build(_columns, model.selection, display: model.display),
      'drive',
    ).style;

    Offset nameOf(LabRowsModel model, Rect rect, String id) {
      final row = _row(
        LabRowsLayout.build(_columns, model.selection, display: model.display),
        id,
      );
      return Offset(
        rect.left + 40,
        rect.top + LabGeometry.ruler + row.y + row.height / 2,
      );
    }

    testWidgets('expands it to a line and folds it back', (tester) async {
      final (model, rect) = await pump(tester);
      await tester.tapAt(nameOf(model, rect, 'drive'));
      await tester.pump();
      expect(model.display.expanded, {'drive'});
      expect(driveStyle(model), LabDrawStyle.line);
      await tester.tapAt(nameOf(model, rect, 'drive'));
      await tester.pump();
      expect(model.display.expanded, isEmpty);
    });

    testWidgets('the strips and the lines are in the Display button only', (
      tester,
    ) async {
      await pump(tester);
      for (final text in ['Lines', 'Plan', 'Actual', 'Both']) {
        expect(find.text(text), findsNothing, reason: text);
      }
    });

    testWidgets('with every level a line, folds it back to its band', (
      tester,
    ) async {
      final (model, rect) = await pump(tester);
      model.setDisplay(model.display.withAllLines(true));
      await tester.pump();
      await tester.tapAt(nameOf(model, rect, 'drive'));
      await tester.pump();
      expect(model.display.expanded, {'drive'});
      expect(driveStyle(model), LabDrawStyle.band);
    });

    testWidgets('does nothing on a row that has one way to be drawn', (
      tester,
    ) async {
      final (model, rect) = await pump(tester);
      final before = model.display;
      await tester.tapAt(nameOf(model, rect, 'shot'));
      await tester.pump();
      expect(identical(model.display, before), isTrue);
    });
  });
}
