import 'dart:math' as math;
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows.dart';
import 'package:rune/widgets/visualizer/lab/lab_rows_painter.dart';
import 'package:rune/widgets/visualizer/lab/lab_segmented.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline_math.dart';
import 'package:rune/widgets/visualizer/lab/lab_tooltip.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';

const _length = 120.0;
const _beat = 1 / 3;
final _sharedText = LabTextCache();

/// A small generated song: four sections, cuts, looks and moments, and one
/// column of every kind.
LabTimelineData _song({
  double songTime = 40,
  double length = _length,
  int song = 0,
  bool playing = false,
}) {
  final n = (length * labBucketRate).ceil();
  Float32List series(double Function(int) f) =>
      Float32List.fromList([for (var b = 0; b < n; b++) f(b)]);
  double t(int b) => b / labBucketRate;

  const columns = [
    LabColumn(
      id: 'sections',
      name: 'Sections',
      group: 'Plan & Actual',
      kind: LabColumnKind.sections,
      hasActual: false,
    ),
    LabColumn(
      id: 'marks',
      name: 'Marks',
      group: 'Plan & Actual',
      kind: LabColumnKind.marks,
    ),
    LabColumn(
      id: 'shot',
      name: 'Shot',
      group: 'Director',
      kind: LabColumnKind.category,
      categories: ['wide', 'close', 'top'],
    ),
    LabColumn(
      id: 'age',
      name: 'Age',
      group: 'Director',
      kind: LabColumnKind.number,
      hi: 8,
      unit: 's',
      plan: LabPlanAvailability.no,
    ),
    LabColumn(
      id: 'locked',
      name: 'Locked',
      group: 'Signals',
      kind: LabColumnKind.bool,
    ),
    LabColumn(
      id: 'drive',
      name: 'Drive',
      group: 'Signals',
      kind: LabColumnKind.number,
    ),
    LabColumn(
      id: 'hits',
      name: 'Hits',
      group: 'Signals',
      kind: LabColumnKind.event,
    ),
    LabColumn(
      id: 'beat',
      name: 'Beat',
      group: 'Signals',
      kind: LabColumnKind.phase,
    ),
    LabColumn(
      id: 'band_low',
      name: 'Band Low',
      group: 'Signals',
      kind: LabColumnKind.number,
      plan: LabPlanAvailability.no,
    ),
  ];

  final drive = series((b) => 0.5 + 0.5 * math.sin(t(b) / 5));
  final hits = series((b) => b % 7 == 0 ? 1 : 0);
  // The actual strips are recorded up to the song time only.
  Float32List upTo(Float32List full) => Float32List.fromList([
    for (var b = 0; b < n; b++) t(b) <= songTime ? full[b] : double.nan,
  ]);

  return LabTimelineData(
    length: length,
    song: song,
    sections: const [
      LabSection(start: 0, end: 20, label: 'intro', tier: LabTier.quiet),
      LabSection(start: 20, end: 60, label: 'verse', tier: LabTier.normal),
      LabSection(start: 60, end: 100, label: 'chorus', tier: LabTier.high),
      LabSection(start: 100, end: 120, label: 'outro', tier: LabTier.quiet),
    ],
    planMarks: [
      const LabMark.cut(
        time: 0,
        shotName: 'street side',
        size: VisualizerShotSize.close,
        first: true,
      ),
      for (var i = 1; i <= 3; i++)
        LabMark.cut(
          time: 57 + i * _beat,
          shotName: 'alley',
          size: VisualizerShotSize.medium,
          fill: i,
        ),
      const LabMark.cut(
        time: 60,
        shotName: 'overhead',
        size: VisualizerShotSize.top,
        strong: true,
        dress: 'flash',
      ),
      const LabMark.look(time: 20, lookName: 'neon'),
      const LabMark.moment(time: 60, strong: true, effect: 'Flare'),
      const LabMark.moment(time: 80, strong: false),
    ],
    actualMarks: const [
      LabMark.cut(
        time: 0,
        shotName: 'street side',
        size: VisualizerShotSize.close,
      ),
      LabMark.manual(time: 12, label: 'Flare'),
      LabMark.jump(time: 30, from: 10),
    ],
    columns: columns,
    planSeries: {
      'shot': series((b) => (b ~/ 100) % 3),
      'locked': series((b) => t(b) > 0.3 ? 1 : 0),
      'drive': drive,
      'hits': hits,
      'beat': series((b) => (t(b) / _beat) % 1),
    },
    actualSeries: {
      'shot': upTo(series((b) => (b ~/ 100) % 3)),
      'age': upTo(series((b) => (t(b) % 6))),
      'locked': upTo(series((b) => t(b) > 0.5 ? 1 : 0)),
      'drive': upTo(drive),
      'hits': upTo(hits),
      'beat': upTo(series((b) => (t(b) / _beat) % 1)),
      'band_low': upTo(series((b) => 0.3)),
    },
    songTime: songTime,
    playing: playing,
  );
}

// The painter of type [T] in the tree.
T _painterOf<T extends CustomPainter>(WidgetTester tester) => tester
    .widgetList<CustomPaint>(find.byType(CustomPaint))
    .map((c) => c.painter)
    .whereType<T>()
    .first;

Future<void> _pump(
  WidgetTester tester,
  LabTimelineData data, {
  required Size size,
  LabRowSelection? selection,
  ValueChanged<double>? onSeek,
  ValueChanged<double>? onScrub,
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    FluentApp(
      // As the app's root: the reveal of its items needs it.
      builder: (context, child) => RevealEffectContext(child: child!),
      home: ScaffoldPage(
        padding: EdgeInsets.zero,
        content: LabTimeline(
          data: data,
          rows: LabRowsModel()
            ..setSelection(selection ?? LabRowSelection.all(data.columns)),
          onSeek: onSeek ?? (_) {},
          onScrub: onScrub,
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  group('zoom and pan', () {
    test('zoom keeps the point under the pointer where it is', () {
      const view = LabView(20, 80);
      final zoomed = labZoomView(view, _length, 50, 0.5);
      expect(zoomed.span, closeTo(30, 1e-9));
      // 50 s was halfway; it still is.
      expect((50 - zoomed.start) / zoomed.span, closeTo(0.5, 1e-9));
    });

    test('the span stays between four seconds and the whole song', () {
      var view = LabView.whole(_length);
      for (var i = 0; i < 40; i++) {
        view = labZoomView(view, _length, 10, 0.5);
      }
      expect(view.span, labMinSpan);
      expect(view.start, greaterThanOrEqualTo(0));
      for (var i = 0; i < 40; i++) {
        view = labZoomView(view, _length, 10, 2);
      }
      expect(view, LabView.whole(_length));
    });

    test('zooming near an edge stays inside the song', () {
      final a = labZoomView(const LabView(0, 60), _length, 0, 0.5);
      expect(a.start, 0);
      final b = labZoomView(const LabView(60, 120), _length, 120, 0.5);
      expect(b.end, _length);
      expect(b.span, 30);
    });

    test('a song shorter than the minimum span shows whole', () {
      final v = labZoomView(const LabView(0, 3), 3, 1, 0.1);
      expect(v, const LabView(0, 3));
    });

    test('pan clamps and the whole song does not move', () {
      const view = LabView(10, 40);
      expect(labPanView(view, _length, -50).start, 0);
      expect(labPanView(view, _length, 500).end, _length);
      expect(labPanView(view, _length, 5), const LabView(15, 45));
      final whole = LabView.whole(_length);
      expect(labPanView(whole, _length, 5), whole);
    });

    test('follow keeps the playhead at 35% of the view', () {
      const view = LabView(0, 20);
      final f = labFollowView(view, _length, 50);
      expect((50 - f.start) / f.span, closeTo(0.35, 1e-9));
      expect(labFollowView(view, _length, 1).start, 0);
      expect(labFollowView(view, _length, 119).end, _length);
    });
  });

  group('aggregation', () {
    final s = Float32List.fromList([1, double.nan, 3, 2]);

    test('numbers take the largest, events the sum, the rest the last', () {
      expect(labAggregate(LabColumnKind.number, s, 0, 4), 3);
      expect(labAggregate(LabColumnKind.event, s, 0, 4), 6);
      expect(labAggregate(LabColumnKind.category, s, 0, 4), 2);
      expect(labAggregate(LabColumnKind.bool, s, 0, 4), 2);
      expect(labAggregate(LabColumnKind.phase, s, 0, 3), 3);
    });

    test('nothing recorded stays NaN', () {
      expect(labAggregate(LabColumnKind.number, s, 1, 2).isNaN, isTrue);
      expect(labAggregate(LabColumnKind.number, s, 2, 2).isNaN, isTrue);
    });

    test('colors follow the kind', () {
      const number = LabColumn(
        id: 'n',
        name: 'n',
        group: 'g',
        kind: LabColumnKind.number,
        lo: 10,
        hi: 20,
      );
      expect(labColorFor(number, 10), labRamp.first);
      expect(labColorFor(number, 99), labRamp.last);
      expect(labColorFor(number, double.nan), isNull);
      const event = LabColumn(
        id: 'e',
        name: 'e',
        group: 'g',
        kind: LabColumnKind.event,
      );
      expect(labColorFor(event, 0), isNull);
      expect(labColorFor(event, 1), isNot(labColorFor(event, 3)));
      const bool = LabColumn(
        id: 'b',
        name: 'b',
        group: 'g',
        kind: LabColumnKind.bool,
      );
      expect(labColorFor(bool, 1), labBoolOn);
    });

    test('times are written with tenths that round down', () {
      expect(labFormatTime(75), '1:15');
      expect(labFormatTime(75.46, tenths: true), '1:15.4');
      expect(labFormatTime(59.96, tenths: true), '0:59.9');
      expect(labFormatTime(-3), '0:00');
    });
  });

  group('rows', () {
    test('the selection survives JSON and falls back to the defaults', () {
      final sel = LabRowSelection.defaults.setPart('age', actual: true);
      final back = LabRowSelection.fromJson(sel.toJson());
      expect(back.actualOn('age'), isTrue);
      expect(back.planOn('shot'), isTrue);
      expect(back.actualOn('sections'), isFalse);
      expect(LabRowSelection.fromJson('not json').planOn('shot'), isTrue);
      expect(LabRowSelection.fromJson(null).planOn('shot'), isTrue);
    });

    test('rows of unavailable strips are left out', () {
      final data = _song();
      final layout = LabRowsLayout.build(
        data.columns,
        LabRowSelection.all(data.columns),
      );
      final names = [
        for (final r in layout.rows)
          if (r.type != LabRowType.group) r.column!.id,
      ];
      expect(
        names,
        containsAll(['sections', 'marks', 'drive', 'band_low', 'age']),
      );
      final bandLow = layout.rows.firstWhere((r) => r.column?.id == 'band_low');
      expect(bandLow.showPlan, isFalse);
      expect(bandLow.showActual, isTrue);
      final age = layout.rows.firstWhere((r) => r.column?.id == 'age');
      expect(age.showPlan, isFalse);
      final sections = layout.rows.firstWhere(
        (r) => r.column?.id == 'sections',
      );
      expect(sections.showActual, isFalse);
      // Group headings come once per group.
      expect(layout.rows.where((r) => r.type == LabRowType.group).length, 3);
    });

    test('the nearest marks are at most four, nearest first', () {
      final marks = [
        for (var i = 0; i < 10; i++)
          LabMark.manual(time: 10 + i * 0.01, label: '$i'),
      ];
      final near = labNearestMarks(marks, 10.03, 0.5);
      expect(near.length, 4);
      expect(near.first.label, '3');
      expect(labNearestMarks(marks, 50, 0.5), isEmpty);
    });

    test('the tooltip words the plan that is missing', () {
      final data = _song();
      final layout = LabRowsLayout.build(
        data.columns,
        LabRowSelection.all(data.columns),
      );
      const size = Size(1000, 600);
      final g = LabGeometry(
        width: size.width,
        height: size.height,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      LabTip? tipOf(String id) {
        final row = layout.rows.firstWhere((r) => r.column?.id == id);
        return labTipAt(
          data: data,
          layout: layout,
          geometry: g,
          x: g.xOf(30),
          y: LabGeometry.ruler + row.y + row.height / 2,
        );
      }

      final no = tipOf('band_low')!.lines.map((l) => l.text);
      expect(no, contains('cannot be predicted'));
      final marks = tipOf('marks')!;
      expect(marks.lines.first.text, 'Plan');
    });
  });

  group('painters', () {
    LabRowsPainter rowsPainter(
      LabTimelineData data, {
      LabGeometry? geometry,
      double now = 10,
      LabTextCache? text,
      LabRowsLayout? layout,
    }) => LabRowsPainter(
      data: data,
      layout: layout ?? LabRowsLayout.empty,
      geometry:
          geometry ??
          LabGeometry(
            width: 1000,
            height: 600,
            view: LabView.whole(_length),
            scrollY: 0,
            length: _length,
          ),
      now: now,
      text: text ?? _sharedText,
      colors: const LabTextColors(
        primary: Color(0xFFFFFFFF),
        secondary: Color(0xA0FFFFFF),
      ),
    );

    test('a cut that wears a transition has a cap, a plain one none', () {
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      final data = _song();
      final layout = LabRowsLayout.build(
        data.columns,
        LabRowSelection.all(data.columns),
      );
      // Nothing else is marked at 90 s. A cut there that wears a fade has a
      // cap two pixels wider on each side than its line.
      final x = g.xOf(90).roundToDouble() + 0.5;
      bool cap(Object? rect, Object? paint) =>
          rect is Rect &&
          paint is Paint &&
          paint.color.toARGB32() == 0xFFFFD27A &&
          (rect.width - 5).abs() < 1e-6 &&
          (rect.height - 2).abs() < 1e-6 &&
          (rect.center.dx - x).abs() < 1e-6;
      PaintPattern hasCap() => paints
        ..something(
          (method, args) => method == #drawRect && cap(args[0], args[1]),
        );
      void Function(Canvas) painting(String? dress) =>
          (canvas) => rowsPainter(
            data.copyWith(
              actualMarks: [
                LabMark.cut(
                  time: 90,
                  shotName: 'alley',
                  size: VisualizerShotSize.medium,
                  dress: dress,
                ),
              ],
            ),
            geometry: g,
            layout: layout,
          ).paint(canvas, const Size(1000, 600));
      expect(painting('fade'), hasCap());
      expect(painting(null), isNot(hasCap()));
    });

    test('the ruler\'s ticks hang from its top edge', () {
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      final painter = rowsPainter(_song(), geometry: g);
      // The whole song is in steps of 15 s on this width.
      final x = g.xOf(15).roundToDouble() + 0.5;
      final tick = Rect.fromLTWH(x - 0.5, 0, 1, 11);
      expect(
        (Canvas canvas) => painter.paint(canvas, const Size(1000, 600)),
        paints
          ..something((method, args) => args.isNotEmpty && args.first == tick),
      );
    });

    test('a zoomed ruler has no bar for where the view is in the song', () {
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: const LabView(30, 60),
        scrollY: 0,
        length: _length,
      );
      final painter = rowsPainter(_song(), geometry: g);
      // The bar ran the plot's width just above the ruler's bottom line.
      final bar = Rect.fromLTWH(g.label, LabGeometry.ruler - 3, g.plotWidth, 2);
      expect(
        (Canvas canvas) => painter.paint(canvas, const Size(1000, 600)),
        paints..everything(
          (method, args) => method != #drawRect || args.first != bar,
        ),
      );
    });

    test('the rows draw no scroll bar of their own', () {
      final data = _song();
      final layout = LabRowsLayout.build(
        data.columns,
        LabRowSelection.all(data.columns),
      );
      final g = LabGeometry(
        width: 1000,
        height: 200,
        view: LabView.whole(_length),
        scrollY: 40,
        length: _length,
      );
      expect(g.maxScroll(layout.contentHeight), greaterThan(40));
      final painter = rowsPainter(data, layout: layout, geometry: g);
      // Its thumb was 3 px wide, 5 px in from the right edge.
      bool thumb(Rect r) => r.left == 1000 - 5 && r.width == 3;
      expect(
        (Canvas canvas) => painter.paint(canvas, const Size(1000, 200)),
        paints..everything(
          (method, args) => method != #drawRect || !thumb(args.first as Rect),
        ),
      );
    });

    test('the rows repaint when the cache lays its texts out again', () {
      final data = _song();
      final cache = LabTextCache();
      final before = rowsPainter(data, text: cache);
      expect(rowsPainter(data, text: cache).shouldRepaint(before), isFalse);
      cache.base = const TextStyle(fontFamily: 'Another');
      // The same cache, with a new generation.
      expect(rowsPainter(data, text: cache).shouldRepaint(before), isTrue);
    });

    test('the rows repaint only for what they draw', () {
      final data = _song();
      final base = rowsPainter(data);
      expect(rowsPainter(data).shouldRepaint(base), isFalse);
      expect(rowsPainter(data, now: 11).shouldRepaint(base), isTrue);
      expect(
        rowsPainter(
          data,
          geometry: LabGeometry(
            width: 1000,
            height: 600,
            view: const LabView(10, 50),
            scrollY: 0,
            length: _length,
          ),
        ).shouldRepaint(base),
        isTrue,
      );
      expect(
        rowsPainter(data.copyWith(songTime: 41)).shouldRepaint(base),
        isTrue,
      );
      expect(
        rowsPainter(
          data,
          layout: LabRowsLayout.build(
            data.columns,
            LabRowSelection.all(data.columns),
          ),
        ).shouldRepaint(base),
        isTrue,
      );
    });

    test('the cursor repaints for the playhead, not for the rows', () {
      final pointer = ValueNotifier<Offset?>(null);
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      final a = LabCursorPainter(geometry: g, now: 10, pointer: pointer);
      expect(
        LabCursorPainter(
          geometry: g,
          now: 10,
          pointer: pointer,
        ).shouldRepaint(a),
        isFalse,
      );
      expect(
        LabCursorPainter(
          geometry: g,
          now: 11,
          pointer: pointer,
        ).shouldRepaint(a),
        isTrue,
      );
      expect(
        LabCursorPainter(
          geometry: g,
          now: 10,
          pointer: pointer,
          resizing: true,
        ).shouldRepaint(a),
        isTrue,
      );
    });

    test('the playhead line lies on whole device pixels around its time', () {
      for (final dpr in const [1.0, 1.5, 2.0, 3.0]) {
        for (final x in const [100.0, 100.2, 100.5, 100.74, 333.3]) {
          final line = LabCursorPainter.playheadLine(x, dpr);
          final left = line.left * dpr;
          final right = (line.left + line.width) * dpr;
          expect(left, closeTo(left.roundToDouble(), 1e-6));
          expect(right, closeTo(right.roundToDouble(), 1e-6));
          expect(line.width, closeTo(1.5, 0.5 / dpr + 1e-9));
          final mid = line.left + line.width / 2;
          expect((mid - x).abs(), lessThanOrEqualTo(0.5 / dpr + 1e-9));
        }
      }
    });

    test('the playhead triangle points at the middle of its line', () {
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      // A time that falls between two pixels.
      const now = 37.3;
      for (final dpr in const [1.0, 2.0]) {
        final painter = LabCursorPainter(
          geometry: g,
          now: now,
          pointer: ValueNotifier<Offset?>(null),
          devicePixelRatio: dpr,
        );
        final line = LabCursorPainter.playheadLine(g.xOf(now), dpr);
        final mid = line.left + line.width / 2;
        // Near the top the triangle is 10 px wide, the same each side of
        // the line's middle.
        expect(
          (Canvas canvas) => painter.paint(canvas, const Size(1000, 600)),
          paints..path(
            includes: [Offset(mid - 4.6, 0.2), Offset(mid + 4.6, 0.2)],
            excludes: [Offset(mid - 5.2, 0.2), Offset(mid + 5.2, 0.2)],
          ),
        );
      }
    });

    test('a plan series that ends early has no value past its end', () {
      final data = _song();
      final column = data.columns.firstWhere((c) => c.id == 'drive');
      final short = LabTimelineData(
        length: _length,
        columns: data.columns,
        planSeries: {
          'drive': Float32List.fromList([0.5, 0.5, 0.5]),
        },
      );
      final layout = LabRowsLayout.build(
        data.columns,
        LabRowSelection.all(data.columns),
      );
      final g = LabGeometry(
        width: 1000,
        height: 600,
        view: LabView.whole(_length),
        scrollY: 0,
        length: _length,
      );
      final row = layout.rows.firstWhere((r) => r.column == column);
      LabTip? tipAt(double time) => labTipAt(
        data: short,
        layout: layout,
        geometry: g,
        x: g.xOf(time),
        y: LabGeometry.ruler + row.y + row.height / 2,
      );
      expect(
        tipAt(0.05)!.lines.firstWhere((l) => l.key == 'Plan').text,
        isNot('—'),
      );
      expect(tipAt(60)!.lines.firstWhere((l) => l.key == 'Plan').text, '—');
    });
  });

  group('text cache', () {
    TextPainter ask(LabTextCache cache, String text) =>
        cache.get(text, size: 11, color: const Color(0xFFFFFFFF));

    test('the same text is laid out once', () {
      final cache = LabTextCache();
      addTearDown(cache.dispose);
      expect(identical(ask(cache, 'a'), ask(cache, 'a')), isTrue);
      expect(cache.length, 1);
    });

    test('the cache stays bounded and what it drops is disposed', () {
      final cache = LabTextCache(maxEntries: 20);
      addTearDown(cache.dispose);
      final all = <TextPainter>[];
      for (var frame = 0; frame < 50; frame++) {
        cache.beginFrame();
        // The frame's texts: the live values change every frame.
        final inFrame = [
          for (var i = 0; i < 12; i++) ask(cache, 'frame $frame value $i'),
        ];
        all.addAll(inFrame);
        // Nothing asked for in this frame is disposed while it paints.
        for (final p in inFrame) {
          expect(p.debugDisposed, isFalse);
        }
        expect(cache.length, lessThanOrEqualTo(2 * (20 + 12)));
      }
      final alive = all.where((p) => !p.debugDisposed).length;
      expect(alive, cache.length);
      expect(all.length - alive, greaterThan(0));
    });

    test('a text asked for again is kept', () {
      final cache = LabTextCache(maxEntries: 5);
      addTearDown(cache.dispose);
      cache.beginFrame();
      final name = ask(cache, 'name');
      for (var frame = 0; frame < 30; frame++) {
        cache.beginFrame();
        expect(identical(ask(cache, 'name'), name), isTrue);
        expect(name.debugDisposed, isFalse);
        for (var i = 0; i < 4; i++) {
          ask(cache, 'value $frame $i');
        }
      }
    });

    test('a text is written in the base style, numbers in tabular figures '
        'of the same font', () {
      final cache = LabTextCache();
      addTearDown(cache.dispose);
      cache.base = const TextStyle(fontFamily: 'App', fontSize: 12);
      final name = cache.get('Drive', color: const Color(0xFFFFFFFF));
      expect(name.text!.style!.fontFamily, 'App');
      expect(name.text!.style!.fontSize, 12);
      expect(name.text!.style!.fontFeatures, isNull);
      final number = cache.get(
        '0:42',
        size: 10,
        color: const Color(0xFFFFFFFF),
        tabular: true,
      );
      expect(number.text!.style!.fontFamily, 'App');
      expect(number.text!.style!.fontFamilyFallback, isNull);
      expect(number.text!.style!.fontSize, 10);
      expect(number.text!.style!.fontFeatures, [
        const FontFeature.tabularFigures(),
      ]);
      // Another style lays the texts out again.
      cache.base = const TextStyle(fontFamily: 'Other', fontSize: 12);
      expect(name.debugDisposed, isTrue);
      expect(
        cache
            .get('Drive', color: const Color(0xFFFFFFFF))
            .text!
            .style!
            .fontFamily,
        'Other',
      );
    });

    test('disposing the cache disposes every painter', () {
      final cache = LabTextCache(maxEntries: 2);
      final painters = <TextPainter>[];
      for (var frame = 0; frame < 3; frame++) {
        cache.beginFrame();
        for (var i = 0; i < 3; i++) {
          painters.add(ask(cache, '$frame $i'));
        }
      }
      cache.dispose();
      for (final p in painters) {
        expect(p.debugDisposed, isTrue);
      }
      expect(cache.length, 0);
    });
  });

  group('play clock', () {
    test('runs on between two readings while the song plays', () {
      final clock = LabPlayClock()..read(10, playing: true, wall: 100);
      expect(clock.timeAt(100.05), closeTo(10.05, 1e-9));
      expect(clock.timeAt(100.1), closeTo(10.1, 1e-9));
    });

    test('holds while the song is paused', () {
      final clock = LabPlayClock()..read(10, playing: false, wall: 100);
      expect(clock.timeAt(101), 10);
    });

    test('a reading off the clock steers it without running back', () {
      // The clock starts 50 ms ahead of the song, which is read ten times a
      // second.
      final clock = LabPlayClock()..read(10.05, playing: true, wall: 100);
      var last = clock.timeAt(100);
      for (var frame = 1; frame <= 60; frame++) {
        final wall = 100 + frame / 60;
        if (frame % 6 == 0) {
          clock.read(10 + (wall - 100), playing: true, wall: wall);
        }
        final shown = clock.timeAt(wall);
        expect(shown, greaterThanOrEqualTo(last));
        last = shown;
      }
      expect(last, closeTo(11, 0.01));
    });

    test('a seek, a pause or another song sets it at once', () {
      final clock = LabPlayClock()..read(10, playing: true, wall: 100);
      clock.read(60, playing: true, wall: 100.1);
      expect(clock.timeAt(100.1), 60);
      clock.read(61, playing: false, wall: 101);
      expect(clock.timeAt(101), 61);
      clock.read(5, playing: true, wall: 102);
      clock.read(5.2, playing: true, wall: 102.1, jump: true);
      expect(clock.timeAt(102.1), 5.2);
    });

    test('stops running on when the readings stop', () {
      final clock = LabPlayClock()..read(10, playing: true, wall: 100);
      expect(clock.timeAt(105), closeTo(10 + LabPlayClock.maxLead, 1e-9));
    });
  });

  group('timeline widget', () {
    for (final size in const [Size(1200, 700), Size(480, 260)]) {
      testWidgets('paints at ${size.width.toInt()} x ${size.height.toInt()}', (
        tester,
      ) async {
        await _pump(tester, _song(), size: size);
        expect(tester.takeException(), isNull);
        expect(find.text('Whole Song'), findsWidgets);
      });
    }

    testWidgets('paints the empty timeline', (tester) async {
      await _pump(
        tester,
        LabTimelineData.empty,
        size: const Size(800, 400),
        selection: LabRowSelection.defaults,
      );
      expect(tester.takeException(), isNull);
    });

    testWidgets('zooms with the buttons and with ctrl and scroll', (
      tester,
    ) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      // Less, the span and more, joined in one frame; the span in the
      // ordinary font.
      final group = find.byType(LabButtonGroup);
      expect(group, findsOneWidget);
      for (final part in [
        find.byIcon(Symbols.remove),
        find.text('Whole Song'),
        find.byIcon(Symbols.add),
      ]) {
        expect(find.descendant(of: group, matching: part), findsOneWidget);
      }
      expect(
        tester.widget<Text>(find.text('Whole Song')).style?.fontFamilyFallback,
        isNull,
      );
      final zoomIn = find.descendant(
        of: group,
        matching: find.byIcon(Symbols.add),
      );
      expect(zoomIn, findsOneWidget);
      await tester.tap(zoomIn);
      await tester.pump();
      expect(tester.takeException(), isNull);
      expect(find.text('Whole Song'), findsNothing);
      for (var i = 0; i < 8; i++) {
        await tester.tap(zoomIn);
        await tester.pump();
      }
      // The shortest span: four seconds.
      expect(find.text('4.0 s'), findsOneWidget);
      // A click on the span shows the whole song.
      await tester.tap(find.text('4.0 s'));
      await tester.pump();
      expect(find.text('Whole Song'), findsWidgets);

      // Ctrl and the wheel zoom around the pointer, with the plot drawn.
      final center = tester.getCenter(find.byType(CustomPaint).last);
      final pointer = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(pointer.hover(center));
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendEventToBinding(pointer.scroll(const Offset(0, -240)));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump(const Duration(milliseconds: 200));
      expect(tester.takeException(), isNull);
      expect(find.text('Whole Song'), findsNothing);
    });

    testWidgets('a tap seeks and a drag scrubs, then seeks on release', (
      tester,
    ) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      await _pump(
        tester,
        _song(),
        size: const Size(1200, 700),
        onSeek: seeks.add,
        onScrub: scrubs.add,
      );
      final paint = find.byType(CustomPaint).last;
      final rect = tester.getRect(paint);
      // The plot runs from 150 px to 12 px short of the right edge.
      double xAt(double t) =>
          rect.left + 150 + t / _length * (rect.width - 162);

      await tester.tapAt(
        Offset(xAt(30), rect.top + 100),
        kind: PointerDeviceKind.mouse,
      );
      expect(seeks.single, closeTo(30, 0.5));

      seeks.clear();
      final gesture = await tester.startGesture(
        Offset(xAt(10), rect.top + 100),
        kind: PointerDeviceKind.mouse,
      );
      await gesture.moveTo(Offset(xAt(50), rect.top + 100));
      await tester.pump();
      expect(scrubs, isNotEmpty);
      expect(scrubs.last, closeTo(50, 0.5));
      expect(seeks, isEmpty);
      await gesture.up();
      expect(seeks.single, closeTo(50, 0.5));

      // A press on the labels does nothing.
      seeks.clear();
      await tester.tapAt(
        Offset(rect.left + 40, rect.top + 100),
        kind: PointerDeviceKind.mouse,
      );
      expect(seeks, isEmpty);
    });

    testWidgets('shows the tooltip over the plot', (tester) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      final paint = find.byType(CustomPaint).last;
      final rect = tester.getRect(paint);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(Offset(rect.left + 400, rect.top + 60));
      await tester.pump();
      expect(tester.takeException(), isNull);
      await mouse.moveTo(Offset(rect.left + 700, rect.top + 200));
      await tester.pump();
      expect(tester.takeException(), isNull);
    });

    testWidgets('follows the song while zoomed and pans off it', (
      tester,
    ) async {
      await _pump(tester, _song(songTime: 10), size: const Size(1200, 700));
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      // The song moves on: follow keeps painting without trouble.
      await _pump(tester, _song(songTime: 90), size: const Size(1200, 700));
      expect(tester.takeException(), isNull);
      // Scrolling sideways pans and switches follow off.
      final center = tester.getCenter(find.byType(CustomPaint).last);
      final pointer = TestPointer(2, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(pointer.hover(center));
      await tester.sendEventToBinding(pointer.scroll(const Offset(100, 0)));
      await tester.pump(const Duration(milliseconds: 200));
      expect(tester.takeException(), isNull);
    });

    testWidgets('keeps the zoom when the length moves a little', (
      tester,
    ) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      expect(find.text('Whole Song'), findsNothing);
      await _pump(tester, _song(length: 121), size: const Size(1200, 700));
      await tester.pump(const Duration(seconds: 1));
      expect(tester.takeException(), isNull);
      expect(find.text('Whole Song'), findsNothing);
    });

    testWidgets('shows another song whole', (tester) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      expect(find.text('Whole Song'), findsNothing);
      await _pump(tester, _song(song: 1), size: const Size(1200, 700));
      expect(find.text('Whole Song'), findsWidgets);
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('shows the whole song when a song arrives', (tester) async {
      await _pump(
        tester,
        LabTimelineData.empty,
        size: const Size(1200, 700),
        selection: LabRowSelection.defaults,
      );
      await _pump(tester, _song(), size: const Size(1200, 700));
      expect(find.text('Whole Song'), findsWidgets);
    });

    testWidgets('the pointer moving shows the label and repaints only its own '
        'layer', (tester) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      CustomPaint paintOf<T extends CustomPainter>() => tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .firstWhere((c) => c.painter is T);
      final rows = paintOf<LabRowsPainter>().painter;
      final cursor = paintOf<LabCursorPainter>().painter;
      final rect = tester.getRect(find.byType(LabTimeline));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(Offset(rect.left + 500, rect.top + 200));
      await tester.pump();
      await mouse.moveTo(Offset(rect.left + 540, rect.top + 230));
      await tester.pump();
      expect(find.byType(LabTipLabel), findsOneWidget);
      // It looks as the app's tooltips do: the theme's box, padding and text.
      final label = find.byType(LabTipLabel);
      final style = TooltipTheme.of(tester.element(label));
      final box = tester.widget<Container>(
        find.descendant(of: label, matching: find.byType(Container)).first,
      );
      expect(box.decoration, style.decoration);
      expect(box.padding, style.padding);
      for (final text in tester.widgetList<Text>(
        find.descendant(of: label, matching: find.byType(Text)),
      )) {
        expect(text.style?.fontFamilyFallback, isNull, reason: text.data);
      }
      // Nothing rebuilt: the same painters are in the tree.
      expect(identical(paintOf<LabRowsPainter>().painter, rows), isTrue);
      expect(identical(paintOf<LabCursorPainter>().painter, cursor), isTrue);
      await mouse.moveTo(Offset(rect.left - 20, rect.top - 20));
      await tester.pump();
      expect(find.byType(LabTipLabel), findsNothing);
    });

    testWidgets('sits on the app\'s card, and writes in the app\'s text and '
        'subtitle colors', (tester) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      final timeline = find.byType(LabTimeline);
      final card = tester.widget<Card>(
        find.descendant(of: timeline, matching: find.byType(Card)),
      );
      expect(card.backgroundColor, isNull);
      expect(card.borderColor, isNull);
      final theme = FluentTheme.of(tester.element(timeline));
      final rows = tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .map((c) => c.painter)
          .whereType<LabRowsPainter>()
          .single;
      expect(
        rows.colors,
        LabTextColors(
          primary: theme.resources.textFillColorPrimary,
          secondary: theme.inactiveColor.withAlpha(160),
        ),
      );
    });

    testWidgets('the playhead runs on between readings while the song plays', (
      tester,
    ) async {
      const size = Size(1200, 700);
      await _pump(tester, _song(songTime: 10, playing: true), size: size);
      final first = _painterOf<LabCursorPainter>(tester).now;
      expect(first, 10);
      final rows = _painterOf<LabRowsPainter>(tester);
      await tester.pump(const Duration(milliseconds: 16));
      await tester.pump(const Duration(milliseconds: 16));
      expect(
        _painterOf<LabCursorPainter>(tester).now - first,
        closeTo(0.032, 1e-6),
      );
      // With the whole song on show, the rows are not painted again.
      expect(_painterOf<LabRowsPainter>(tester).shouldRepaint(rows), isFalse);
      // The values by the names stay with the reading.
      expect(_painterOf<LabRowsPainter>(tester).now, 10);
      // Paused, the playhead stays where the song is.
      await _pump(tester, _song(songTime: 10.5, playing: false), size: size);
      await tester.pump(const Duration(milliseconds: 100));
      expect(_painterOf<LabCursorPainter>(tester).now, 10.5);
    });

    testWidgets('the toolbar is not built again at every frame', (
      tester,
    ) async {
      const size = Size(1200, 700);
      await _pump(tester, _song(songTime: 10, playing: true), size: size);
      final toolbar = tester.widget(find.byType(LabButtonGroup));
      final first = _painterOf<LabCursorPainter>(tester).now;
      await tester.pump(const Duration(milliseconds: 16));
      await tester.pump(const Duration(milliseconds: 16));
      // The playhead ran on; the toolbar is the same widget.
      expect(_painterOf<LabCursorPainter>(tester).now, greaterThan(first));
      expect(
        identical(tester.widget(find.byType(LabButtonGroup)), toolbar),
        isTrue,
      );
      await _pump(tester, _song(songTime: 10.1), size: size);
    });

    testWidgets('a followed view moves at every frame while the song plays', (
      tester,
    ) async {
      await _pump(
        tester,
        _song(songTime: 40, playing: true),
        size: const Size(1200, 700),
      );
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      double start() => _painterOf<LabRowsPainter>(tester).geometry.view.start;
      final before = start();
      await tester.pump(const Duration(milliseconds: 16));
      expect(start() - before, closeTo(0.016, 1e-6));
      // Stopped, so that the test ends with no frames to come, and past the
      // buttons' tooltips.
      await _pump(tester, _song(songTime: 40.1), size: const Size(1200, 700));
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('a trackpad slide keeps to one axis and glides on after it', (
      tester,
    ) async {
      // Short, so that the rows scroll.
      await _pump(tester, _song(), size: const Size(1200, 300));
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      double start() => _painterOf<LabRowsPainter>(tester).geometry.view.start;
      double rows() => _painterOf<LabRowsPainter>(tester).geometry.scrollY;
      final at = tester.getCenter(find.byType(CustomPaint).last);
      final pad = TestPointer(3, PointerDeviceKind.trackpad);
      var ms = 0;
      Duration step() => Duration(milliseconds: ms += 16);

      Future<void> slide(List<Offset> steps) async {
        await tester.sendEventToBinding(
          pad.panZoomStart(at, timeStamp: step()),
        );
        var pan = Offset.zero;
        for (final each in steps) {
          pan += each;
          await tester.sendEventToBinding(
            pad.panZoomUpdate(at, pan: pan, timeStamp: step()),
          );
          await tester.pump(const Duration(milliseconds: 16));
        }
        await tester.sendEventToBinding(pad.panZoomEnd(timeStamp: step()));
        await tester.pump();
      }

      // Up and a little sideways: the rows scroll, the song stays.
      final song = start();
      await slide(List.filled(4, const Offset(2, -12)));
      expect(rows(), greaterThan(0));
      expect(start(), song);
      await tester.pumpAndSettle();

      // Sideways and a little down, once mostly down: the song moves, the
      // rows stay.
      final scrolled = rows();
      final before = start();
      await slide([
        ...List.filled(4, const Offset(-30, 6)),
        const Offset(-2, 14),
        ...List.filled(4, const Offset(-30, 6)),
      ]);
      final lifted = start();
      expect(lifted, greaterThan(before));
      expect(rows(), scrolled);
      // It glides on after the fingers lift, then stops.
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(), greaterThan(lifted));
      await tester.pumpAndSettle();
      final settled = start();
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(), settled);
      expect(rows(), scrolled);
    });

    testWidgets('the rows that scroll have the app\'s scroll bar, which the '
        'wheel moves and a drag on it moves, seeking nothing', (tester) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      // Short, so that the rows scroll.
      await _pump(
        tester,
        _song(),
        size: const Size(1200, 300),
        onSeek: seeks.add,
        onScrub: scrubs.add,
      );
      final bar = find.descendant(
        of: find.byType(LabTimeline),
        matching: find.byType(Scrollbar),
      );
      expect(bar, findsOneWidget);
      LabRowsPainter rows() => _painterOf<LabRowsPainter>(tester);
      final g = rows().geometry;
      final fixed = rows().layout.fixed(g.rowsHeight);
      final position = tester.widget<Scrollbar>(bar).controller!.position;
      // It runs along the rows under the pinned ones, and shows that much.
      expect(position.viewportDimension, g.rowsHeight - fixed.height);
      expect(
        position.maxScrollExtent,
        rows().layout.contentHeight - g.rowsHeight,
      );
      expect(position.maxScrollExtent, greaterThan(0));

      // The wheel over the plot moves it, and the rows with it.
      final box = tester.getRect(bar);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: box.center);
      await tester.sendEventToBinding(
        PointerScrollEvent(
          position: box.center,
          scrollDelta: const Offset(0, 30),
        ),
      );
      await tester.pump();
      expect(position.pixels, 30);
      expect(rows().geometry.scrollY, 30);
      await tester.pumpAndSettle();

      // Over the bar, it comes in; a drag on it moves the rows down.
      final thumb = Offset(
        box.right - 5,
        box.top + LabGeometry.ruler + fixed.height + 4 + 12,
      );
      await mouse.moveTo(thumb);
      await tester.pumpAndSettle();
      await mouse.down(thumb);
      await tester.pump();
      await mouse.moveTo(thumb + const Offset(0, 30));
      await tester.pump();
      await mouse.up();
      await tester.pumpAndSettle();
      expect(position.pixels, greaterThan(30));
      expect(rows().geometry.scrollY, position.pixels);
      expect(seeks, isEmpty);
      expect(scrubs, isEmpty);
      await mouse.removePointer();
      // Past the bar's wait before it narrows.
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('fingers that stop before they lift do not glide', (
      tester,
    ) async {
      await _pump(tester, _song(), size: const Size(1200, 700));
      for (var i = 0; i < 3; i++) {
        await tester.tap(find.byIcon(Symbols.add));
        await tester.pump();
      }
      double start() => _painterOf<LabRowsPainter>(tester).geometry.view.start;
      final at = tester.getCenter(find.byType(CustomPaint).last);
      final pad = TestPointer(4, PointerDeviceKind.trackpad);
      await tester.sendEventToBinding(
        pad.panZoomStart(at, timeStamp: Duration.zero),
      );
      for (var i = 1; i <= 6; i++) {
        await tester.sendEventToBinding(
          pad.panZoomUpdate(
            at,
            pan: Offset(-30.0 * i, 0),
            timeStamp: Duration(milliseconds: 16 * i),
          ),
        );
      }
      await tester.sendEventToBinding(
        pad.panZoomEnd(timeStamp: const Duration(milliseconds: 400)),
      );
      await tester.pump();
      final lifted = start();
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(), lifted);
    });
  });

  group('touch', () {
    // A finger with its own clock, so that its speed is what the test says.
    Future<_Finger> finger(WidgetTester tester, int id, Offset at) async {
      final f = _Finger(tester, TestPointer(id, PointerDeviceKind.touch));
      await f.down(at);
      return f;
    }

    LabGeometry geometry(WidgetTester tester) =>
        _painterOf<LabRowsPainter>(tester).geometry;
    double start(WidgetTester tester) => geometry(tester).view.start;

    // The point on the name of [id]'s row, and a point on its plot at [x].
    Offset nameOf(WidgetTester tester, String id) {
      final p = _painterOf<LabRowsPainter>(tester);
      final row = p.layout.rows.firstWhere((r) => r.column?.id == id);
      final rect = tester.getRect(find.byType(CustomPaint).last);
      return Offset(
        rect.left + 40,
        rect.top +
            LabGeometry.ruler +
            row.y +
            row.height / 2 -
            p.geometry.scrollY,
      );
    }

    Offset plotAt(WidgetTester tester, double x, {String row = 'drive'}) =>
        Offset(x, nameOf(tester, row).dy);

    // A point in the ruler, at [x].
    Offset rulerAt(WidgetTester tester, double x) => Offset(
      x,
      tester.getRect(find.byType(CustomPaint).last).top + LabGeometry.ruler / 2,
    );

    Future<LabRowsModel> pump(
      WidgetTester tester, {
      Size size = const Size(1200, 700),
      List<double>? seeks,
      List<double>? scrubs,
      List<(double?, bool)>? widths,
      bool zoomed = false,
    }) async {
      final data = _song();
      final model = LabRowsModel()
        ..setSelection(LabRowSelection.all(data.columns));
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(
        FluentApp(
          builder: (context, child) => RevealEffectContext(child: child!),
          home: ScaffoldPage(
            padding: EdgeInsets.zero,
            content: LabTimeline(
              data: data,
              rows: model,
              onSeek: (t) => seeks?.add(t),
              onScrub: (t) => scrubs?.add(t),
              onLabelWidth: (w, {required save}) => widths?.add((w, save)),
            ),
          ),
        ),
      );
      await tester.pump();
      if (zoomed) {
        for (var i = 0; i < 3; i++) {
          await tester.tap(find.byIcon(Symbols.add));
          await tester.pump();
        }
      }
      return model;
    }

    // Far from the playhead, so that a touch pans.
    double awayFromPlayhead(WidgetTester tester) {
      final g = geometry(tester);
      final x = g.xOf(40);
      return x + 250 < g.label + g.plotWidth ? x + 250 : x - 250;
    }

    testWidgets('a sideways drag pans the song and seeks nothing', (
      tester,
    ) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      await pump(tester, seeks: seeks, scrubs: scrubs, zoomed: true);
      final before = start(tester);
      final x = awayFromPlayhead(tester);
      final f = await finger(tester, 1, plotAt(tester, x));
      var at = plotAt(tester, x);
      for (var i = 0; i < 8; i++) {
        at += const Offset(-20, 1);
        await f.move(at);
      }
      // The content follows the finger: the song moves on.
      expect(start(tester), greaterThan(before));
      await f.pause(300);
      await f.up();
      await tester.pump();
      expect(seeks, isEmpty);
      expect(scrubs, isEmpty);
      // Panning by hand turns Follow off.
      expect(
        tester.widget<ToggleButton>(find.byType(ToggleButton)).checked,
        false,
      );
    });

    testWidgets('a vertical drag scrolls the rows and keeps the song', (
      tester,
    ) async {
      final seeks = <double>[];
      await pump(
        tester,
        size: const Size(1200, 300),
        seeks: seeks,
        zoomed: true,
      );
      final before = start(tester);
      final x = awayFromPlayhead(tester);
      final rect = tester.getRect(find.byType(CustomPaint).last);
      var at = Offset(x, rect.bottom - 20);
      final f = await finger(tester, 1, at);
      for (var i = 0; i < 6; i++) {
        at += const Offset(2, -25);
        await f.move(at);
      }
      expect(geometry(tester).scrollY, greaterThan(0));
      expect(start(tester), before);
      await f.pause(300);
      await f.up();
      await tester.pump();
      expect(seeks, isEmpty);
      // Past the bar's wait before it narrows.
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('a fling glides on and a finger that stopped does not', (
      tester,
    ) async {
      await pump(tester, zoomed: true);
      final x = awayFromPlayhead(tester);

      Future<double> drag({required bool stop}) async {
        var at = plotAt(tester, x);
        final f = await finger(tester, stop ? 2 : 1, at);
        for (var i = 0; i < 6; i++) {
          at += const Offset(-30, 0);
          await f.move(at);
        }
        if (stop) await f.pause(400);
        await f.up();
        await tester.pump();
        return start(tester);
      }

      final lifted = await drag(stop: false);
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(tester), greaterThan(lifted));
      await tester.pumpAndSettle();
      final settled = start(tester);
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(tester), settled);

      // Pan back, then stop before the lift: it stays where it is.
      final f = await finger(tester, 3, plotAt(tester, x));
      var back = plotAt(tester, x);
      for (var i = 0; i < 6; i++) {
        back += const Offset(30, 0);
        await f.move(back);
      }
      await f.up();
      await tester.pumpAndSettle();
      final before = start(tester);
      expect(before, lessThan(settled));
      final stopped = await drag(stop: true);
      await tester.pump(const Duration(milliseconds: 100));
      expect(start(tester), stopped);
    });

    testWidgets('a tap seeks once at the time tapped', (tester) async {
      final seeks = <double>[];
      await pump(tester, seeks: seeks);
      final g = geometry(tester);
      final f = await finger(tester, 1, plotAt(tester, g.xOf(90)));
      await f.pause(50);
      await f.up();
      await tester.pump(const Duration(seconds: 1));
      expect(seeks.single, closeTo(90, 0.5));
    });

    testWidgets('a tap on a level\'s name switches it to a line', (
      tester,
    ) async {
      final model = await pump(tester);
      final f = await finger(tester, 1, nameOf(tester, 'drive'));
      await f.pause(50);
      await f.up();
      await tester.pump();
      expect(model.display.expanded, {'drive'});
    });

    testWidgets('a long press on a name opens the row menu', (tester) async {
      final seeks = <double>[];
      final model = await pump(tester, seeks: seeks);
      final f = await finger(tester, 1, nameOf(tester, 'drive'));
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 10));
      await tester.pumpAndSettle();
      expect(find.text('Hide Row'), findsOneWidget);
      await f.up();
      await tester.pumpAndSettle();
      expect(seeks, isEmpty);
      expect(model.display.expanded, isEmpty);
    });

    testWidgets('a long press on the plot shows the label until it lifts', (
      tester,
    ) async {
      final seeks = <double>[];
      await pump(tester, seeks: seeks, zoomed: true);
      final before = start(tester);
      final x = awayFromPlayhead(tester);
      final f = await finger(tester, 1, plotAt(tester, x));
      expect(find.byType(LabTipLabel), findsNothing);
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 10));
      expect(find.byType(LabTipLabel), findsOneWidget);
      // It follows the finger, and the song does not pan.
      await f.move(plotAt(tester, x - 120));
      await tester.pump();
      expect(find.byType(LabTipLabel), findsOneWidget);
      expect(start(tester), before);
      await f.up();
      await tester.pump();
      expect(find.byType(LabTipLabel), findsNothing);
      expect(seeks, isEmpty);
    });

    testWidgets(
      'a drag from the playhead\'s head in the ruler scrubs and seeks '
      'where it ends',
      (tester) async {
        final seeks = <double>[];
        final scrubs = <double>[];
        await pump(tester, seeks: seeks, scrubs: scrubs);
        final g = geometry(tester);
        // A little off it, within a finger's reach.
        final f = await finger(tester, 1, rulerAt(tester, g.xOf(40) + 14));
        await f.move(rulerAt(tester, g.xOf(55)));
        await f.move(rulerAt(tester, g.xOf(70)));
        expect(scrubs.last, closeTo(70, 0.5));
        expect(seeks, isEmpty);
        await f.up();
        await tester.pump();
        expect(seeks.single, closeTo(70, 0.5));
      },
    );

    testWidgets('a slide that starts near the playhead in the rows pans', (
      tester,
    ) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      await pump(tester, seeks: seeks, scrubs: scrubs, zoomed: true);
      final before = start(tester);
      final x = geometry(tester).xOf(40) + 10;
      final f = await finger(tester, 1, plotAt(tester, x));
      var at = plotAt(tester, x);
      for (var i = 0; i < 8; i++) {
        at += const Offset(-20, 1);
        await f.move(at);
      }
      expect(start(tester), greaterThan(before));
      await f.pause(300);
      await f.up();
      await tester.pump();
      expect(scrubs, isEmpty);
      expect(seeks, isEmpty);
    });

    testWidgets('two fingers pinch around their focal point and slide', (
      tester,
    ) async {
      final seeks = <double>[];
      await pump(tester, seeks: seeks);
      // Follow would zoom around the playhead.
      await tester.tap(find.byType(ToggleButton));
      await tester.pump();
      final g = geometry(tester);
      final cx = g.xOf(80);
      final focalTime = g.timeOf(cx);
      final a = await finger(tester, 1, plotAt(tester, cx - 60));
      final b = await finger(tester, 2, plotAt(tester, cx + 60));
      var gap = 60.0;
      while (gap < 180) {
        gap += 20;
        await a.move(plotAt(tester, cx - gap));
        await b.move(plotAt(tester, cx + gap));
      }
      final zoomed = geometry(tester);
      expect(zoomed.view.span, lessThan(g.view.span / 2));
      expect(zoomed.timeOf(cx), closeTo(focalTime, 0.5));

      // Both fingers sideways: the song goes with them.
      final before = zoomed.view.start;
      var shift = 0.0;
      while (shift < 160) {
        shift += 20;
        await a.move(plotAt(tester, cx - gap + shift));
        await b.move(plotAt(tester, cx + gap + shift));
      }
      expect(start(tester), lessThan(before));
      await a.up();
      await b.up();
      await tester.pump();
      expect(seeks, isEmpty);
    });

    testWidgets('a second finger cancels a pending tap, hold and drag', (
      tester,
    ) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      await pump(tester, seeks: seeks, scrubs: scrubs);
      final g = geometry(tester);

      // A tap that a second finger spoils.
      var a = await finger(tester, 1, plotAt(tester, g.xOf(90)));
      var b = await finger(tester, 2, plotAt(tester, g.xOf(95)));
      await a.up();
      await b.up();
      await tester.pump();
      expect(seeks, isEmpty);

      // A hold on a name that a second finger spoils.
      a = await finger(tester, 3, nameOf(tester, 'drive'));
      b = await finger(tester, 4, plotAt(tester, g.xOf(95)));
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 10));
      await tester.pumpAndSettle();
      expect(find.text('Hide Row'), findsNothing);
      await a.up();
      await b.up();

      // A drag of the playhead that a second finger spoils.
      a = await finger(tester, 5, rulerAt(tester, g.xOf(40)));
      await a.move(rulerAt(tester, g.xOf(60)));
      b = await finger(tester, 6, plotAt(tester, g.xOf(95)));
      await a.up();
      await b.up();
      await tester.pump();
      expect(seeks, isEmpty);
    });

    testWidgets('a finger near the label column\'s edge resizes it', (
      tester,
    ) async {
      final widths = <(double?, bool)>[];
      final seeks = <double>[];
      await pump(tester, widths: widths, seeks: seeks);
      final edge = geometry(tester).label;
      // Ten pixels off: a mouse would miss it, a finger does not.
      final f = await finger(tester, 1, plotAt(tester, edge + 10));
      await f.move(plotAt(tester, edge + 50));
      await f.move(plotAt(tester, edge + 70));
      expect(widths.last.$1, closeTo(edge + 60, 0.5));
      expect(widths.last.$2, false);
      await f.up();
      await tester.pump();
      expect(widths.last.$2, true);
      expect(seeks, isEmpty);
    });

    testWidgets('a mouse drag on the plot still scrubs', (tester) async {
      final seeks = <double>[];
      final scrubs = <double>[];
      await pump(tester, seeks: seeks, scrubs: scrubs, zoomed: true);
      final x = awayFromPlayhead(tester);
      final before = start(tester);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.down(plotAt(tester, x));
      await mouse.moveTo(plotAt(tester, x - 80));
      await tester.pump();
      expect(scrubs, isNotEmpty);
      await mouse.up();
      await tester.pump();
      expect(seeks, hasLength(1));
      expect(start(tester), before);
      await mouse.removePointer();
      await tester.pump(const Duration(seconds: 1));
    });
  });
}

// A touch with a clock of its own: the events carry the time the test says,
// as a real touch's do, so that its speed is known.
class _Finger {
  _Finger(this.tester, this.pointer);

  final WidgetTester tester;
  final TestPointer pointer;
  int _ms = 0;

  Duration _step(int by) => Duration(milliseconds: _ms += by);

  Future<void> down(Offset at) =>
      tester.sendEventToBinding(pointer.down(at, timeStamp: _step(0)));

  Future<void> move(Offset to) async {
    await tester.sendEventToBinding(pointer.move(to, timeStamp: _step(16)));
    await tester.pump(const Duration(milliseconds: 16));
  }

  // The finger stays where it is for [ms].
  Future<void> pause(int ms) async {
    _ms += ms;
  }

  Future<void> up() =>
      tester.sendEventToBinding(pointer.up(timeStamp: _step(16)));
}
