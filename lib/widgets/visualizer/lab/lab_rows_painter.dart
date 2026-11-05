import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';

import 'lab_data.dart';
import 'lab_display.dart';
import 'lab_draw_style.dart';
import 'lab_rows.dart';
import 'lab_timeline_math.dart';

const _stripHeight = 10.0;
const _textStripHeight = 14.0;
const _laneHeight = 30.0;
const _stripGap = 2.0;
const _markHeight = 22.0;
const _markGap = 3.0;
const _sectionsRowHeight = 18.0;
const _groupHeight = 20.0;

bool _isLane(LabDrawStyle style) =>
    style == LabDrawStyle.line || style == LabDrawStyle.steps;

double _stripHeightOf(LabDrawStyle style) => switch (style) {
  LabDrawStyle.line || LabDrawStyle.steps => _laneHeight,
  LabDrawStyle.blocks || LabDrawStyle.numbers => _textStripHeight,
  _ => _stripHeight,
};

enum LabRowType { group, sections, marks, parameter }

/// A row of the timeline, laid out from the top of the rows.
class LabRow {
  const LabRow({
    required this.type,
    required this.y,
    required this.height,
    this.column,
    this.groupName = '',
    this.showPlan = false,
    this.showActual = false,
    this.style = LabDrawStyle.band,
    this.level = false,
  });

  final LabRowType type;
  final double y;
  final double height;
  final LabColumn? column;
  final String groupName;
  final bool showPlan;
  final bool showActual;

  /// How the row draws its values.
  final LabDrawStyle style;

  /// Whether the row is a level, which is as tall as a lane whether it is
  /// drawn as a band or a line, with one strip or two, so that switching
  /// between them does not move the rows.
  final bool level;

  /// Whether the plan and the actual share one lane, drawn over each other.
  bool get isLane => _isLane(style);

  // The space between two strips of the row.
  double get _gap => type == LabRowType.marks ? _markGap : _stripGap;

  /// The height of a strip, or of the lane. A row is as tall as all the
  /// strips it can have, whichever of them are shown, so the strips that are
  /// shown share its height and switching strips does not move the rows.
  double get stripHeight {
    if (isLane) return _laneHeight;
    final n = math.max(1, partCount);
    return (height - (n - 1) * _gap) / n;
  }

  /// Where the [i]th strip starts, from the top of the row. The parts of a
  /// lane all start at the top.
  double stripTop(int i) => isLane ? 0 : i * (stripHeight + _gap);

  /// The strip at [y] from the top of the row.
  int partAt(double y) {
    if (isLane || partCount < 2) return 0;
    return (y / (stripHeight + _gap)).floor().clamp(0, partCount - 1);
  }

  /// Where the middle of the mark of the [i]th part is, from the top of the
  /// row: the plan and the actual of a lane are marked apart.
  double markMiddle(int i) {
    if (!isLane) return stripTop(i) + stripHeight / 2;
    if (partCount < 2) return height / 2;
    return i == 0 ? height * 0.27 : height * 0.73;
  }

  /// The strips of the row from the top: true for the plan, false for the
  /// actual.
  int get partCount => (showPlan ? 1 : 0) + (showActual ? 1 : 0);

  /// Whether the [i]th strip is the plan's.
  bool isPlanPart(int i) => showPlan && i == 0;
}

/// The rows the selection shows, and the height they take together.
///
/// The pinned rows come first, under a heading of their own. As many of them
/// as fit stay fixed under the ruler ([fixed]); the others go at the top of
/// the rows that scroll. Every row has its place in one run of heights, so a
/// row that scrolls keeps its [LabRow.y] whether or not the pinned rows are
/// fixed.
class LabRowsLayout {
  const LabRowsLayout(
    this.rows,
    this.contentHeight, {
    this.pinnedCount = 0,
    this.pinnedEnd = 0,
  });

  static const empty = LabRowsLayout([], 8);

  /// The most of the rows' height that the fixed rows can take.
  static const fixedShare = 0.4;

  final List<LabRow> rows;
  final double contentHeight;

  /// How many of the first [rows] belong to the pinned block (its heading
  /// too), and where the block ends.
  final int pinnedCount;
  final double pinnedEnd;

  /// The layout of the [columns] that [selection] has on: the pinned ones
  /// first, in pin order, under a "Pinned" heading, then the others under
  /// their group headings. A pinned column that is not shown is not drawn.
  /// With [LabDisplay.hideEmpty], the columns in [empty] are left out,
  /// the pinned ones excepted. The [display] says which strips each row
  /// draws, and in which style.
  factory LabRowsLayout.build(
    List<LabColumn> columns,
    LabRowSelection selection, {
    LabDisplay display = const LabDisplay(),
    Set<String> empty = const {},
  }) {
    final rows = <LabRow>[];
    var y = 4.0;

    void heading(String name) {
      rows.add(
        LabRow(
          type: LabRowType.group,
          y: y,
          height: _groupHeight,
          groupName: name,
        ),
      );
      y += _groupHeight;
    }

    void add(LabColumn c, bool showPlan, bool showActual) {
      // The strips the column can have, shown or not: the row is as tall as
      // them all.
      final n = math.max(
        1,
        (c.plan == LabPlanAvailability.yes ? 1 : 0) + (c.hasActual ? 1 : 0),
      );
      final style = display.styleOf(c);
      final double h;
      final LabRowType type;
      switch (c.kind) {
        case LabColumnKind.sections:
          type = LabRowType.sections;
          h = _sectionsRowHeight;
        case LabColumnKind.marks:
          type = LabRowType.marks;
          h = n * _markHeight + (n - 1) * _markGap;
        default:
          type = LabRowType.parameter;
          final strips = _isLane(style) ? 1 : n;
          h = labExpandable(c)
              ? _laneHeight
              : math.max(
                  16,
                  strips * _stripHeightOf(style) + (strips - 1) * _stripGap,
                );
      }
      rows.add(
        LabRow(
          type: type,
          y: y,
          height: h,
          column: c,
          showPlan: showPlan,
          showActual: showActual,
          style: style,
          level: type == LabRowType.parameter && labExpandable(c),
        ),
      );
      y += h + (type == LabRowType.marks ? 8 : 5);
    }

    final byId = {for (final c in columns) c.id: c};
    for (final id in selection.pins) {
      final c = byId[id];
      if (c == null) continue;
      final showPlan = display.showsPlan(selection, c);
      final showActual = display.showsActual(selection, c);
      if (!showPlan && !showActual) continue;
      if (rows.isEmpty) heading('Pinned');
      add(c, showPlan, showActual);
    }
    final pinnedCount = rows.length;
    final pinnedEnd = pinnedCount == 0 ? 0.0 : y;

    String? lastGroup;
    for (final c in columns) {
      if (selection.isPinned(c.id)) continue;
      final showPlan = display.showsPlan(selection, c);
      final showActual = display.showsActual(selection, c);
      if (!showPlan && !showActual) continue;
      if (display.hideEmpty && empty.contains(c.id)) continue;
      if (c.group != lastGroup) {
        heading(c.group);
        lastGroup = c.group;
      }
      add(c, showPlan, showActual);
    }
    return LabRowsLayout(
      rows,
      y + 8,
      pinnedCount: pinnedCount,
      pinnedEnd: pinnedEnd,
    );
  }

  /// The rows that stay fixed under the ruler when the rows have
  /// [rowsHeight] to show in: the first [count] of [rows], which take
  /// [height]. They are the pinned block, as much of it as fits in
  /// [fixedShare] of [rowsHeight], by whole rows; none when not even its
  /// first row does.
  ({int count, double height}) fixed(double rowsHeight) {
    if (pinnedCount < 2) return (count: 0, height: 0);
    final cap = rowsHeight * fixedShare;
    var count = pinnedCount;
    var height = pinnedEnd;
    while (count > 1 && height > cap) {
      count--;
      height = rows[count].y;
    }
    return count < 2 ? (count: 0, height: 0) : (count: count, height: height);
  }

  /// Where [viewY], from the top of the rows' viewport, is in the run of
  /// heights: the fixed rows do not move, the others are [scrollY] down.
  double contentY(double viewY, double scrollY, double rowsHeight) =>
      viewY < fixed(rowsHeight).height ? viewY : viewY + scrollY;

  /// The row under [viewY], from the top of the rows' viewport, not counting
  /// group headings. Over the fixed rows only they are met; below them only
  /// the rows that scroll.
  LabRow? rowAtView(double viewY, double scrollY, double rowsHeight) {
    final f = fixed(rowsHeight);
    if (viewY < f.height) return _rowIn(viewY, 0, f.count);
    return _rowIn(viewY + scrollY, f.count, rows.length);
  }

  /// The row at [contentY] (from the top of the rows) and the position in it,
  /// not counting group headings.
  LabRow? rowAt(double contentY) => _rowIn(contentY, 0, rows.length);

  LabRow? _rowIn(double contentY, int from, int to) {
    for (var i = from; i < to; i++) {
      final r = rows[i];
      if (r.type == LabRowType.group) continue;
      if (contentY >= r.y - 2 && contentY <= r.y + r.height + 2) return r;
    }
    return null;
  }
}

/// Where things are on the timeline's canvas, for the painter and for the
/// pointer.
class LabGeometry {
  const LabGeometry({
    required this.width,
    required this.height,
    required this.view,
    required this.scrollY,
    required this.length,
    this.labelWidth,
  });

  static const ruler = 28.0;
  static const padRight = 12.0;

  /// The narrowest and the widest the label column can be.
  static const minLabel = 88.0;
  static const maxLabel = 420.0;

  /// The width of the label column when the user has not set one.
  static double defaultLabel(double width) => width < 560 ? 108 : 150;

  /// [wanted] kept in the range the label column can have in [width]: at
  /// least [minLabel], and at most [maxLabel] or half of the width.
  static double clampLabel(double wanted, double width) {
    final hi = math.max(minLabel, math.min(maxLabel, width * 0.5));
    return wanted.clamp(minLabel, hi).toDouble();
  }

  final double width;
  final double height;
  final LabView view;
  final double scrollY;
  final double length;

  /// The label column's width as the user set it; none for the default.
  final double? labelWidth;

  /// The width of the label column.
  double get label => clampLabel(labelWidth ?? defaultLabel(width), width);

  double get plotWidth => math.max(10, width - label - padRight);

  double get rowsHeight => math.max(0, height - ruler);

  double xOf(double time) =>
      label + (time - view.start) / view.span * plotWidth;

  double timeOf(double x) => view.start + (x - label) / plotWidth * view.span;

  /// The largest scroll of the rows for [contentHeight].
  double maxScroll(double contentHeight) =>
      math.max(0, contentHeight - rowsHeight);

  /// The bucket a time falls in.
  static int bucketOf(double time) => (time * labBucketRate).floor();

  @override
  bool operator ==(Object other) =>
      other is LabGeometry &&
      width == other.width &&
      height == other.height &&
      view == other.view &&
      scrollY == other.scrollY &&
      length == other.length &&
      labelWidth == other.labelWidth;

  @override
  int get hashCode =>
      Object.hash(width, height, view, scrollY, length, labelWidth);
}

/// TextPainters that are laid out once for the same text and style, so that a
/// frame lays out only what changed.
///
/// The cache keeps two generations. A painter that is asked for again is moved
/// to the current one; [beginFrame], which a paint calls first, drops the old
/// generation once the current one has grown past [maxEntries], and disposes
/// what it drops. Nothing a paint has asked for is ever disposed during that
/// paint, and what the cache holds stays near [maxEntries] however many
/// different texts come, as the label column's values do.
class LabTextCache {
  LabTextCache({this.maxEntries = 300});

  /// How many painters the current generation may hold when a frame begins.
  final int maxEntries;

  var _map = <String, TextPainter>{};
  var _old = <String, TextPainter>{};

  TextStyle _base = const TextStyle();

  /// The style the texts are written in, as the app's: their font, and
  /// their size when [get] gives none. A new one lays every text out again.
  set base(TextStyle style) {
    if (style == _base) return;
    _base = style;
    generation++;
    dispose();
  }

  /// Counts the new styles: a painter that drew with an older one is out of
  /// date though the cache is the same object.
  int generation = 0;

  /// How many painters the cache holds.
  int get length => _map.length + _old.length;

  /// Starts a paint: when the current generation is full, the old one is
  /// disposed and the current one becomes it. Call it before the paint asks
  /// for any text.
  void beginFrame() {
    if (_map.length <= maxEntries) return;
    for (final p in _old.values) {
      p.dispose();
    }
    _old = _map;
    _map = {};
  }

  TextPainter get(
    String text, {
    double? size,
    required Color color,
    FontWeight? weight,
    bool tabular = false,
    double? maxWidth,
  }) {
    final key =
        '$size|${weight?.value}|${color.toARGB32()}|$tabular|$maxWidth|$text';
    final cached = _map[key];
    if (cached != null) return cached;
    final aged = _old.remove(key);
    if (aged != null) return _map[key] = aged;
    final painter = TextPainter(
      text: TextSpan(
        text: text,
        style: _base.copyWith(
          fontSize: size,
          color: color,
          fontWeight: weight,
          // Numbers keep their places as they change.
          fontFeatures: tabular ? const [FontFeature.tabularFigures()] : null,
        ),
      ),
      maxLines: 1,
      ellipsis: '…',
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: maxWidth ?? double.infinity);
    return _map[key] = painter;
  }

  void dispose() {
    for (final p in _map.values) {
      p.dispose();
    }
    for (final p in _old.values) {
      p.dispose();
    }
    _map.clear();
    _old.clear();
  }
}

/// The colors the timeline writes in, which the theme gives: the text's, and
/// the dimmer one of what is said about it.
@immutable
class LabTextColors {
  const LabTextColors({required this.primary, required this.secondary});

  final Color primary;
  final Color secondary;

  @override
  bool operator ==(Object other) =>
      other is LabTextColors &&
      other.primary == primary &&
      other.secondary == secondary;

  @override
  int get hashCode => Object.hash(primary, secondary);
}

Color _tierColor(LabTier tier) => switch (tier) {
  LabTier.quiet => const Color(0xFF4A78C2),
  LabTier.normal => const Color(0xFF2A9D8F),
  LabTier.high => const Color(0xFFD0573F),
};

/// The color of a tier, as the sections row and the tooltip show it.
Color labTierColor(LabTier tier) => _tierColor(tier);

double _cutHeight(String size) => switch (size) {
  'wide' => 1,
  'top' => 0.9,
  'medium' => 0.8,
  'close' => 0.65,
  'detail' => 0.5,
  _ => 0.7,
};

const _burstColor = Color(0xFFE3A34B);
const _lookColor = Color(0xFFF39AC7);
const _momentColor = Color(0xFFFFD27A);
const _manualColor = Color(0xFFFF9F43);
const _jumpColor = Color(0x99C8C8D2);

// The least width of a row's name that the value may leave it.
const _minNameWidth = 56.0;

/// Paints the ruler and the rows of the timeline; the app's scroll bar is
/// over them. The playhead and the pointer's line are [LabCursorPainter]'s,
/// in a layer of their own over this one, so that the pointer moving repaints
/// nothing here.
/// It repaints as the song plays, so it keeps no allocation per bucket or per
/// mark: the paints and the texts are reused.
class LabRowsPainter extends CustomPainter {
  LabRowsPainter({
    required this.data,
    required this.layout,
    required this.geometry,
    required this.now,
    required this.text,
    required this.colors,
  }) : textGeneration = text.generation;

  final LabTimelineData data;
  final LabRowsLayout layout;
  final LabGeometry geometry;

  /// The song time the playhead is at (while scrubbing, the scrub's), which
  /// the label column's values are read at.
  final double now;
  final LabTextCache text;

  /// The cache's generation when this painter was made.
  final int textGeneration;
  final LabTextColors colors;

  static final Paint _fill = Paint()..isAntiAlias = false;
  static final Paint _aa = Paint();
  static final Paint _stroke = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.2;

  LabGeometry get g => geometry;
  LabView get view => geometry.view;

  void _rect(Canvas canvas, double x, double y, double w, double h, Color c) {
    _fill.color = c;
    canvas.drawRect(Rect.fromLTWH(x, y, w, h), _fill);
  }

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) return;
    text.beginFrame();
    _paintRuler(canvas, size);

    final fixed = layout.fixed(g.rowsHeight);
    if (fixed.count > 0) {
      canvas.save();
      canvas.clipRect(
        Rect.fromLTWH(0, LabGeometry.ruler, size.width, fixed.height),
      );
      canvas.translate(0, LabGeometry.ruler);
      _paintRows(canvas, 0, fixed.count, 0, fixed.height);
      canvas.restore();
    }

    // The rest scrolls under the fixed rows, and is cut off at them.
    final below = LabGeometry.ruler + fixed.height;
    canvas.save();
    canvas.clipRect(
      Rect.fromLTWH(0, below, size.width, g.rowsHeight - fixed.height),
    );
    canvas.translate(0, LabGeometry.ruler - g.scrollY);
    _paintRows(
      canvas,
      fixed.count,
      layout.rows.length,
      g.scrollY + fixed.height,
      g.scrollY + g.rowsHeight,
    );
    canvas.restore();
    if (fixed.count > 0 && g.scrollY > 0) {
      _rect(canvas, 0, below - 1, size.width, 1, const Color(0x1AFFFFFF));
    }
  }

  // Paints the rows [from] to [to] that are in the heights [top] to [bottom],
  // from where the canvas is moved to.
  void _paintRows(Canvas canvas, int from, int to, double top, double bottom) {
    // The section boundaries run across the rows.
    for (final s in data.sections) {
      if (s.start <= view.start || s.start >= view.end) continue;
      _rect(
        canvas,
        g.xOf(s.start).roundToDouble(),
        top,
        1,
        bottom - top,
        const Color(0x0DFFFFFF),
      );
    }

    for (var i = from; i < to; i++) {
      final r = layout.rows[i];
      if (r.y + r.height < top || r.y > bottom) continue;
      if (r.type == LabRowType.group) {
        final p = text.get(r.groupName, color: colors.secondary);
        p.paint(canvas, Offset(12, r.y + 11 - p.height / 2));
        continue;
      }
      _paintLabel(canvas, r);
      canvas.save();
      canvas.clipRect(
        Rect.fromLTWH(g.label, r.y - 2, g.plotWidth, r.height + 4),
      );
      switch (r.type) {
        case LabRowType.sections:
          _paintSections(canvas, r);
        case LabRowType.marks:
          for (var i = 0; i < r.partCount; i++) {
            _paintMarks(
              canvas,
              r.isPlanPart(i) ? data.planMarks : data.actualMarks,
              r.y + r.stripTop(i),
              r.stripHeight,
            );
          }
        case LabRowType.parameter:
          final c = r.column!;
          if (r.isLane) {
            _paintLane(canvas, r, c);
            break;
          }
          for (var i = 0; i < r.partCount; i++) {
            final plan = r.isPlanPart(i);
            _paintStrip(
              canvas,
              c,
              plan ? data.planSeries[c.id] : data.actualSeries[c.id],
              r.y + r.stripTop(i),
              r.stripHeight,
              plan,
              r.style,
            );
          }
        case LabRowType.group:
          break;
      }
      canvas.restore();
    }
  }

  // ---------- ruler ----------

  static const List<double> _steps = [0.5, 1, 2, 5, 10, 15, 30, 60, 120];

  void _paintRuler(Canvas canvas, Size size) {
    const rulerH = LabGeometry.ruler;
    final sp = view.span;
    final w = g.plotWidth;
    _rect(canvas, 0, 0, size.width, rulerH, const Color(0x08FFFFFF));
    _rect(canvas, 0, rulerH - 1, size.width, 1, const Color(0x14FFFFFF));

    var step = _steps.last;
    for (final s in _steps) {
      if (s / sp * w >= 70) {
        step = s;
        break;
      }
    }

    final first = (view.start / step).ceil();
    for (var k = first; k * step <= view.end + 1e-6; k++) {
      final t = k * step;
      final x = g.xOf(t).roundToDouble() + 0.5;
      if (x < g.label - 0.5 || x > g.label + w + 0.5) continue;
      // A tick hangs from the line above the ruler.
      _rect(canvas, x - 0.5, 0, 1, 11, const Color(0x47FFFFFF));
      final p = text.get(
        labFormatTime(t, tenths: step < 1),
        color: colors.secondary,
        tabular: true,
      );
      if (x + 4 + p.width < g.label + w + 8) {
        p.paint(canvas, Offset(x + 4, 11 - p.height / 2));
      }
    }
  }

  // ---------- labels ----------

  double _currentValue(LabColumn c) {
    final b = LabGeometry.bucketOf(now);
    final a = data.actualSeries[c.id];
    if (a != null && a.isNotEmpty) {
      final i = (b - 1).clamp(0, a.length - 1);
      if (!a[i].isNaN) return a[i];
    }
    final p = data.planSeries[c.id];
    if (p != null && p.isNotEmpty) {
      final i = math.max(b, 0);
      if (i < p.length) return p[i];
    }
    return double.nan;
  }

  void _paintLabel(Canvas canvas, LabRow r) {
    final c = r.column!;
    final midY = r.y + r.height / 2;
    var value = '';
    if (r.type == LabRowType.sections) {
      value = data.sectionAt(now)?.label ?? '';
    } else if (r.type == LabRowType.parameter) {
      value = labFormatValue(c, _currentValue(c));
    }
    final label = g.label;
    // The name starts at 12 and the value ends 18 before the edge, where the
    // P and A marks are. The value goes before the name is squeezed out.
    final room = math.max(10.0, label - 30);
    var name = text.get(c.name, color: colors.primary, maxWidth: room);
    TextPainter? v;
    if (value.isNotEmpty && r.type != LabRowType.marks) {
      final shown = text.get(
        value,
        color: colors.secondary,
        tabular: true,
        maxWidth: math.min(64.0, room * 0.42),
      );
      final nameRoom = room - shown.width - 6;
      if (name.width <= nameRoom) {
        v = shown;
      } else if (nameRoom >= _minNameWidth) {
        v = shown;
        name = text.get(c.name, color: colors.primary, maxWidth: nameRoom);
      }
    }
    name.paint(canvas, Offset(12, midY - name.height / 2));
    if (v != null) {
      v.paint(canvas, Offset(label - 18 - v.width, midY - v.height / 2));
    }
    if (r.type != LabRowType.sections) {
      for (var i = 0; i < r.partCount; i++) {
        final cy = r.y + r.markMiddle(i);
        final l = text.get(
          r.isPlanPart(i) ? 'P' : 'A',
          size: 9,
          color: colors.secondary,
        );
        l.paint(canvas, Offset(label - 11 - l.width / 2, cy - l.height / 2));
      }
    }
  }

  // ---------- sections ----------

  void _paintSections(Canvas canvas, LabRow r) {
    for (final s in data.sections) {
      if (s.end < view.start || s.start > view.end) continue;
      final xa = g.xOf(s.start) + 1;
      final xb = g.xOf(s.end) - 1;
      _aa.color = _tierColor(s.tier).withValues(alpha: 0.5);
      canvas.drawRRect(
        RRect.fromLTRBR(
          xa,
          r.y + 1,
          xa + math.max(1, xb - xa),
          r.y + r.height - 1,
          const Radius.circular(3),
        ),
        _aa,
      );
      final lx = math.max(xa, g.label) + 5;
      if (xb - lx > 8) {
        final p = text.get(s.label, color: colors.primary);
        if (xb - lx > p.width + 4) {
          p.paint(canvas, Offset(lx, r.y + r.height / 2 - p.height / 2));
        }
      }
    }
  }

  // ---------- strips ----------

  void _paintStrip(
    Canvas canvas,
    LabColumn c,
    Float32List? series,
    double y,
    double h,
    bool isPlan,
    LabDrawStyle style,
  ) {
    _rect(canvas, g.label, y, g.plotWidth, h, const Color(0x09FFFFFF));
    if (series == null || series.isEmpty) return;
    switch (style) {
      case LabDrawStyle.blocks:
        _paintRuns(canvas, c, series, y, h, isPlan, named: true);
      case LabDrawStyle.numbers:
        _paintRuns(canvas, c, series, y, h, isPlan, named: false);
      case LabDrawStyle.ticks:
        _paintTicks(canvas, c, series, y, h, isPlan);
      case LabDrawStyle.aroundRest:
        _paintColors(canvas, c, series, y, h, isPlan, labRestColor);
      case LabDrawStyle.band:
      case LabDrawStyle.fill:
      case LabDrawStyle.line:
      case LabDrawStyle.steps:
        _paintColors(canvas, c, series, y, h, isPlan, labColorFor);
    }
  }

  // What a pixel column or a bucket needs of the series in view.
  ({double w, double sp, int rate, int len, int b0, int b1}) _span(
    Float32List series,
  ) {
    final sp = view.span;
    final len = math.min(series.length, data.bucketCount);
    return (
      w: g.plotWidth,
      sp: sp,
      rate: labBucketRate,
      len: len,
      b0: math.max(0, (view.start * labBucketRate).floor()),
      b1: math.min(len, (view.end * labBucketRate).ceil()),
    );
  }

  // The buckets of the pixel column [px]: from [ba] up to the end [bb]
  // returned, or null when the song has none there.
  (int, int)? _bucketsOf(int px, int len) {
    final w = g.plotWidth;
    final ta = view.start + px / w * view.span;
    final tb = view.start + (px + 1) / w * view.span;
    final ba = math.max(0, (ta * labBucketRate).floor());
    if (ba >= len) return null;
    final bb = math.min(len, math.max(ba + 1, (tb * labBucketRate).floor()));
    return (ba, bb);
  }

  Color _dim(Color col, bool isPlan) =>
      isPlan ? col.withValues(alpha: col.a * 0.6) : col;

  // A color for each value: for each bucket where they are wide enough, else
  // for each pixel column.
  void _paintColors(
    Canvas canvas,
    LabColumn c,
    Float32List series,
    double y,
    double h,
    bool isPlan,
    Color? Function(LabColumn, double) colorOf,
  ) {
    final x0 = g.label;
    final s = _span(series);
    final w = s.w;
    final pxPerBucket = w / (s.sp * s.rate);

    Color? runC;
    var runX = 0.0;
    void flush(double endX) {
      final col = runC;
      if (col == null) return;
      final a = math.max(x0, runX);
      final b = math.min(x0 + w, endX);
      if (b > a) _rect(canvas, a, y, b - a, h, _dim(col, isPlan));
    }

    if (pxPerBucket >= 1) {
      for (var b = s.b0; b <= s.b1; b++) {
        final col = b < s.b1 ? colorOf(c, series[b]) : null;
        final x = x0 + (b / s.rate - view.start) / s.sp * w;
        if (col != runC) {
          flush(x);
          runC = col;
          runX = x;
        }
      }
      return;
    }

    final wide = w.ceil();
    for (var px = 0; px <= wide; px++) {
      Color? col;
      final range = px < wide ? _bucketsOf(px, s.len) : null;
      if (range != null) {
        col = colorOf(c, labAggregate(c.kind, series, range.$1, range.$2));
      }
      if (col != runC) {
        flush(x0 + px);
        runC = col;
        runX = x0 + px.toDouble();
      }
    }
  }

  // Events as ticks where the buckets are wide enough; else as the density of
  // the events in each pixel column.
  void _paintTicks(
    Canvas canvas,
    LabColumn c,
    Float32List series,
    double y,
    double h,
    bool isPlan,
  ) {
    final x0 = g.label;
    final s = _span(series);
    final w = s.w;
    final pxPerBucket = w / (s.sp * s.rate);
    if (pxPerBucket >= 2) {
      final tw = math.max(1.0, math.min(2.0, pxPerBucket * 0.3));
      for (var b = s.b0; b < s.b1; b++) {
        final v = series[b];
        if (!(v > 0)) continue;
        _rect(
          canvas,
          x0 + (b / s.rate - view.start) / s.sp * w,
          y,
          tw,
          h,
          _dim(labEventColor(v), isPlan),
        );
      }
      return;
    }
    final wide = w.ceil();
    for (var px = 0; px < wide; px++) {
      final range = _bucketsOf(px, s.len);
      if (range == null) continue;
      final sum = labAggregate(c.kind, series, range.$1, range.$2);
      if (!(sum > 0)) continue;
      _rect(
        canvas,
        x0 + px,
        y,
        1,
        h,
        _dim(
          const Color(0xFFFFFFFF).withValues(alpha: labDensityAlpha(sum)),
          isPlan,
        ),
      );
    }
  }

  // Runs of the same category, or of the same number, as blocks that say
  // what they are where they are wide enough.
  void _paintRuns(
    Canvas canvas,
    LabColumn c,
    Float32List series,
    double y,
    double h,
    bool isPlan, {
    required bool named,
  }) {
    final x0 = g.label;
    final s = _span(series);
    final wide = s.w.ceil();
    final range = c.hi - c.lo;
    final same = (range == 0 ? 1.0 : range.abs()) * 1e-3;

    var runValue = double.nan;
    var runX = 0;
    void flush(int endPx) {
      if (runValue.isNaN || endPx <= runX) return;
      final xa = x0 + runX;
      final xb = math.min(x0 + s.w, x0 + endPx);
      final fill = named
          ? labColorFor(c, runValue)
          : (isPlan ? const Color(0x14FFFFFF) : const Color(0x26FFFFFF));
      if (fill == null || xb <= xa) return;
      _rect(canvas, xa, y, xb - xa, h, _dim(fill, isPlan));
      if (!named) _rect(canvas, xa, y, 1, h, const Color(0x59FFFFFF));
      final room = (xb - xa - 4).floorToDouble();
      if (room < 8) return;
      final label = labFormatValue(c, runValue);
      final Color color;
      if (named) {
        color = fill.computeLuminance() > 0.5
            ? const Color(0xFF14141C)
            : const Color(0xFFFFFFFF);
      } else {
        color = isPlan ? colors.secondary : colors.primary;
      }
      final p = text.get(
        label,
        size: 10,
        color: isPlan ? color.withValues(alpha: 0.8) : color,
        tabular: !named,
        maxWidth: room,
      );
      // A number is written whole or not at all.
      if (!named && p.width > room) return;
      canvas.save();
      canvas.clipRect(Rect.fromLTRB(xa, y, xb, y + h));
      p.paint(
        canvas,
        Offset(xa + (xb - xa - p.width) / 2, y + (h - p.height) / 2),
      );
      canvas.restore();
    }

    for (var px = 0; px <= wide; px++) {
      var v = double.nan;
      final buckets = px < wide ? _bucketsOf(px, s.len) : null;
      if (buckets != null) {
        v = labAggregate(c.kind, series, buckets.$1, buckets.$2);
      }
      final continues =
          !v.isNaN &&
          !runValue.isNaN &&
          (named
              ? v.toInt() == runValue.toInt()
              : (v - runValue).abs() <= same);
      if (continues) continue;
      flush(px);
      runValue = v;
      runX = px;
    }
  }

  // ---------- lines ----------

  static final Path _path = Path();
  static final Paint _line = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1.4
    ..strokeJoin = StrokeJoin.round
    ..strokeCap = StrokeCap.butt;

  static const _laneInset = 3.0;
  static const _lineColor = Color(0xFF38BFE0);

  // The plan and the actual of a row that draws a line or steps, over each
  // other in one lane.
  void _paintLane(Canvas canvas, LabRow r, LabColumn c) {
    final y = r.y;
    final h = r.stripHeight;
    _rect(canvas, g.label, y, g.plotWidth, h, const Color(0x09FFFFFF));
    _rect(
      canvas,
      g.label,
      y + h - _laneInset,
      g.plotWidth,
      1,
      const Color(0x14FFFFFF),
    );
    for (var i = 0; i < r.partCount; i++) {
      final plan = r.isPlanPart(i);
      final series = plan ? data.planSeries[c.id] : data.actualSeries[c.id];
      if (series == null || series.isEmpty) continue;
      _line.color = plan ? _lineColor.withValues(alpha: 0.55) : _lineColor;
      if (r.style == LabDrawStyle.steps) {
        _stepPath(c, series, y, h);
      } else {
        _linePath(c, series, y, h);
      }
      canvas.drawPath(_path, _line);
    }
  }

  // Where a value is in the lane that starts at [y] and is [h] high.
  double _laneY(LabColumn c, double v, double y, double h) {
    final double f;
    if (c.kind == LabColumnKind.category) {
      final n = c.categories.length;
      f = n > 1 ? v / (n - 1) : 0.0;
    } else if (c.kind == LabColumnKind.phase) {
      f = v;
    } else {
      final range = c.hi - c.lo;
      f = range == 0 ? 0.0 : (v - c.lo) / range;
    }
    final inner = h - 2 * _laneInset;
    return y + _laneInset + (1 - math.min(1.0, math.max(0.0, f))) * inner;
  }

  // The line of a number or a phase in [_path]: a polyline through the
  // buckets where they are wide enough, else a segment from the least to the
  // largest of each pixel column, joined to the one before.
  void _linePath(LabColumn c, Float32List series, double y, double h) {
    _path.reset();
    final x0 = g.label;
    final s = _span(series);
    final w = s.w;
    final phase = c.kind == LabColumnKind.phase;
    if (w / (s.sp * s.rate) >= 1) {
      var open = false;
      var prev = double.nan;
      for (var b = s.b0; b < s.b1; b++) {
        final v = series[b];
        if (v.isNaN) {
          open = false;
          continue;
        }
        final x = x0 + (b / s.rate - view.start) / s.sp * w;
        final py = _laneY(c, v, y, h);
        // A phase that wraps from the end of its cycle starts a new line.
        if (!open || (phase && v < prev - 0.5)) {
          _path.moveTo(x, py);
          open = true;
        } else {
          _path.lineTo(x, py);
        }
        prev = v;
      }
      return;
    }
    final wide = w.ceil();
    var open = false;
    var lastY = 0.0;
    for (var px = 0; px < wide; px++) {
      final buckets = _bucketsOf(px, s.len);
      var lo = double.infinity;
      var hi = double.negativeInfinity;
      if (buckets != null) {
        for (var b = buckets.$1; b < buckets.$2; b++) {
          final v = series[b];
          if (v.isNaN) continue;
          if (v < lo) lo = v;
          if (v > hi) hi = v;
        }
      }
      if (lo > hi) {
        open = false;
        continue;
      }
      final x = x0 + px + 0.5;
      final ya = _laneY(c, lo, y, h);
      final yb = _laneY(c, hi, y, h);
      // Enter at the end nearer to where the line was.
      final firstA = !open || (ya - lastY).abs() <= (yb - lastY).abs();
      final first = firstA ? ya : yb;
      final second = firstA ? yb : ya;
      if (!open) {
        _path.moveTo(x, first);
        open = true;
      } else {
        _path.lineTo(x, first);
      }
      if (second != first) _path.lineTo(x, second);
      lastY = second;
    }
  }

  // The steps of a category in [_path]: level until the value changes, then
  // straight to the next level.
  void _stepPath(LabColumn c, Float32List series, double y, double h) {
    _path.reset();
    final x0 = g.label;
    final s = _span(series);
    final wide = s.w.ceil();
    var open = false;
    var lastY = 0.0;
    for (var px = 0; px < wide; px++) {
      final buckets = _bucketsOf(px, s.len);
      final v = buckets == null
          ? double.nan
          : labAggregate(c.kind, series, buckets.$1, buckets.$2);
      if (v.isNaN) {
        if (open) _path.lineTo(x0 + px, lastY);
        open = false;
        continue;
      }
      final py = _laneY(c, v, y, h);
      if (!open) {
        _path.moveTo(x0 + px, py);
        open = true;
      } else if (py != lastY) {
        _path.lineTo(x0 + px, lastY);
        _path.lineTo(x0 + px, py);
      }
      lastY = py;
    }
    if (open) _path.lineTo(x0 + wide, lastY);
  }

  // ---------- marks ----------

  void _paintMarks(Canvas canvas, List<LabMark> list, double y, double h) {
    final x0 = g.label;
    final w = g.plotWidth;
    _rect(canvas, x0, y, w, h, const Color(0x09FFFFFF));
    _rect(canvas, x0, y + h - 1, w, 1, const Color(0x29FFFFFF));
    if (list.isEmpty) return;

    final i0 = labLowerBound<LabMark>(list, view.start - 2, (m) => m.time);
    final i1 = labLowerBound<LabMark>(list, view.end + 2, (m) => m.time);

    // A burst is a run of cuts with rising fill numbers; a bracket joins it.
    double burstA = double.nan;
    double burstB = double.nan;
    var lastFill = 0;
    final baseline = y + h - 3;
    void closeBurst() {
      if (!burstA.isNaN && !burstB.isNaN) {
        final xa = g.xOf(burstA).roundToDouble();
        final xb = g.xOf(burstB).roundToDouble();
        _burstPaint(canvas, xa, xb, baseline);
      }
      burstA = double.nan;
      burstB = double.nan;
    }

    for (var i = i0; i < i1; i++) {
      final m = list[i];
      if (m.kind == LabMarkKind.cut) {
        if (m.fill > 0) {
          if (burstA.isNaN || m.fill <= lastFill) {
            closeBurst();
            burstA = m.time;
          }
          burstB = m.time;
        } else {
          closeBurst();
        }
        lastFill = m.fill;
      }
      final x = g.xOf(m.time).roundToDouble() + 0.5;
      switch (m.kind) {
        case LabMarkKind.cut:
          final hh = _cutHeight(m.size) * (h - 6);
          final w = m.strong ? 2.0 : 1.0;
          _rect(
            canvas,
            x - w / 2,
            baseline - hh,
            w,
            hh,
            const Color(0xE0FFFFFF),
          );
          // A cut that wears a transition has a cap the color of the moment
          // that chose it.
          if (m.dress != null) {
            _rect(canvas, x - w / 2 - 2, baseline - hh, w + 4, 2, _momentColor);
          }
        case LabMarkKind.look:
          _rect(canvas, x - 1, y, 2, 6, _lookColor);
        case LabMarkKind.moment:
          final r = m.strong ? 3.2 : 2.4;
          final center = Offset(x, y + h * 0.42);
          if (m.effect != null) {
            _aa
              ..color = _momentColor
              ..style = PaintingStyle.fill;
            canvas.drawCircle(center, r, _aa);
          } else {
            _stroke.color = _momentColor;
            canvas.drawCircle(center, r, _stroke);
          }
        case LabMarkKind.manual:
          _rect(canvas, x - 0.75, y, 1.5, h, _manualColor);
          _aa
            ..color = _manualColor
            ..style = PaintingStyle.fill;
          canvas.drawCircle(Offset(x, y + 3.5), 3, _aa);
        case LabMarkKind.jump:
          for (var dy = 0.0; dy < h; dy += 6) {
            _rect(
              canvas,
              x - 0.5,
              y + dy,
              1,
              math.min(3.0, h - dy),
              _jumpColor,
            );
          }
      }
    }
    closeBurst();
  }

  void _burstPaint(Canvas canvas, double xa, double xb, double baseline) {
    final y = baseline + 1;
    _rect(canvas, xa, y, math.max(1.0, xb - xa + 1), 1.5, _burstColor);
    _rect(canvas, xa, y - 2, 1, 2, _burstColor);
    _rect(canvas, xb, y - 2, 1, 2, _burstColor);
  }

  @override
  bool shouldRepaint(LabRowsPainter old) =>
      !identical(old.data, data) ||
      old.data.actualRevision != data.actualRevision ||
      !identical(old.layout, layout) ||
      old.geometry != geometry ||
      old.now != now ||
      !identical(old.text, text) ||
      old.textGeneration != textGeneration ||
      old.colors != colors;
}

/// Where the pointer is over the timeline, in the canvas's coordinates.
typedef LabPointer = ValueListenable<Offset?>;

/// Paints the playhead, the line under the pointer and the highlight of the
/// label column's edge, over the rows. It repaints when the pointer moves
/// (it listens to [pointer]) and when the playhead does, and nothing else
/// does.
class LabCursorPainter extends CustomPainter {
  LabCursorPainter({
    required this.geometry,
    required this.now,
    required this.pointer,
    this.resizing = false,
    this.playheadColor = const Color(0xFFFFFFFF),
    this.devicePixelRatio = 1,
  }) : super(repaint: pointer);

  final LabGeometry geometry;

  /// The song time the playhead is at (while scrubbing, the scrub's).
  final double now;
  final LabPointer pointer;

  /// Whether the edge of the label column is being dragged.
  final bool resizing;
  final Color playheadColor;

  /// The screen's pixels to a logical pixel, to put the playhead's edges on
  /// whole pixels.
  final double devicePixelRatio;

  /// The playhead's line at [x]: 1.5 logical pixels wide, in whole device
  /// pixels of [devicePixelRatio], with its edges on them. Its middle is
  /// `left + width / 2`, where the triangle on top points.
  static ({double left, double width}) playheadLine(
    double x,
    double devicePixelRatio,
  ) {
    final dpr = devicePixelRatio > 0 ? devicePixelRatio : 1.0;
    final pixels = math.max(1.0, (1.5 * dpr).roundToDouble());
    final left = (x * dpr - pixels / 2).roundToDouble();
    return (left: left / dpr, width: pixels / dpr);
  }

  static final Paint _fill = Paint()..isAntiAlias = false;
  static final Paint _aa = Paint();
  static final Path _playhead = Path();

  LabGeometry get g => geometry;

  void _rect(Canvas canvas, double x, double y, double w, double h, Color c) {
    _fill.color = c;
    canvas.drawRect(Rect.fromLTWH(x, y, w, h), _fill);
  }

  /// The half width of the hit zone on the label column's edge.
  static const edgeHalf = 3.0;

  /// Whether [x] is on the edge between the label column and the plot.
  static bool onEdge(double x, LabGeometry g) =>
      (x - g.label).abs() <= edgeHalf;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0 || size.height <= 0) return;
    _paintPlayhead(canvas, size);
    final at = pointer.value;
    final hot = resizing || (at != null && onEdge(at.dx, g));
    if (hot) {
      _rect(
        canvas,
        g.label.roundToDouble() - 1,
        0,
        2,
        size.height,
        const Color(0x59C9B6E6),
      );
    }
    final hx = at?.dx;
    if (!hot && hx != null && hx >= g.label && hx <= g.label + g.plotWidth) {
      _rect(
        canvas,
        hx.roundToDouble() - 0.5,
        LabGeometry.ruler,
        1,
        size.height - LabGeometry.ruler,
        const Color(0x40FFFFFF),
      );
    }
  }

  void _paintPlayhead(Canvas canvas, Size size) {
    final px = g.xOf(now);
    if (px < g.label - 1 || px > g.label + g.plotWidth + 1) return;
    // The line and the triangle are one shape around one middle, so that the
    // triangle points at the line's middle.
    final line = playheadLine(px, devicePixelRatio);
    final mid = line.left + line.width / 2;
    _playhead
      ..reset()
      ..addRect(Rect.fromLTWH(line.left, 0, line.width, size.height))
      ..moveTo(mid - 5, 0)
      ..lineTo(mid + 5, 0)
      ..lineTo(mid, 6)
      ..close();
    _aa
      ..color = playheadColor
      ..style = PaintingStyle.fill;
    canvas.drawPath(_playhead, _aa);
  }

  @override
  bool shouldRepaint(LabCursorPainter old) =>
      old.geometry != geometry ||
      old.now != now ||
      old.resizing != resizing ||
      old.playheadColor != playheadColor ||
      old.devicePixelRatio != devicePixelRatio ||
      !identical(old.pointer, pointer);
}
