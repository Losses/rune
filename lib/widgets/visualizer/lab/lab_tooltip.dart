import 'dart:math' as math;
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';

import 'lab_data.dart';
import 'lab_rows_painter.dart';
import 'lab_timeline_math.dart';

// A mark this many pixels from the pointer is described.
const _hoverReach = 5.0;

/// One line of the label that shows what is under the pointer.
class LabTipLine {
  const LabTipLine(this.text, {this.key, this.dim = false, this.bold = false});

  /// The caption before the text, such as Plan and Actual.
  final String? key;
  final String text;
  final bool dim;
  final bool bold;
}

/// What the label shows: a time, then the lines.
class LabTip {
  const LabTip(this.time, this.lines);

  final String time;
  final List<LabTipLine> lines;
}

String _clock(double t) => labFormatTime(t, tenths: true);

/// What a mark is, in a line. [onCut] says that a moment's effect is the
/// transition a cut wears (see [labMomentOnCut]).
String labDescribeMark(LabMark m, {bool onCut = false}) {
  switch (m.kind) {
    case LabMarkKind.cut:
      final extra = [
        if (m.first) 'first',
        if (m.fill > 0) 'burst ${m.fill}',
        if (m.strong) 'strong',
        ?m.dress,
      ].join(', ');
      // A mark has no index to number an unnamed shot or look by.
      final shot = m.shotName.isEmpty ? 'unnamed shot' : m.shotName;
      return 'Cut ${_clock(m.time)}  $shot (${m.size})'
          '${extra.isEmpty ? '' : '  $extra'}';
    case LabMarkKind.look:
      final look = m.lookName.isEmpty ? 'unnamed look' : m.lookName;
      return 'Look ${_clock(m.time)}  $look';
    case LabMarkKind.moment:
      final effect = switch (m.effect) {
        null => 'no effect',
        final e when onCut => '$e on the cut',
        final e => '$e over the picture',
      };
      return 'Moment ${_clock(m.time)}  ${m.strong ? 'strong' : 'medium'}'
          '  $effect';
    case LabMarkKind.manual:
      return 'By hand ${_clock(m.time)}  ${m.label}';
    case LabMarkKind.jump:
      return 'Seek ${_clock(m.from)} → ${_clock(m.time)}';
  }
}

/// Whether the effect of [moment] is the transition that a cut of [list]
/// wears: a moment that plays when the director cuts dresses the cut, which
/// comes at the same time, rather than playing over the picture.
bool labMomentOnCut(List<LabMark> list, LabMark moment) {
  if (moment.kind != LabMarkKind.moment || moment.effect == null) return false;
  final from = labLowerBound<LabMark>(list, moment.time, (m) => m.time);
  for (var i = from; i < list.length && list[i].time == moment.time; i++) {
    final m = list[i];
    if (m.kind == LabMarkKind.cut && m.dress != null) return true;
  }
  return false;
}

/// The marks of [list] within [reach] seconds of [time], nearest first, at
/// most four.
List<LabMark> labNearestMarks(List<LabMark> list, double time, double reach) {
  final from = labLowerBound<LabMark>(list, time - reach, (m) => m.time);
  final near = <LabMark>[];
  for (var i = from; i < list.length && list[i].time <= time + reach; i++) {
    near.add(list[i]);
  }
  near.sort((a, b) => (a.time - time).abs().compareTo((b.time - time).abs()));
  return near.take(4).toList();
}

/// The label for the point ([x], [y]) of the timeline's canvas, or null when
/// the pointer is not over the plot.
LabTip? labTipAt({
  required LabTimelineData data,
  required LabRowsLayout layout,
  required LabGeometry geometry,
  required double x,
  required double y,
}) {
  if (x < geometry.label || y < LabGeometry.ruler) return null;
  final time = math.min(math.max(geometry.timeOf(x), 0.0), geometry.length);
  final lines = <LabTipLine>[];
  final viewY = y - LabGeometry.ruler;
  final row = layout.rowAtView(viewY, geometry.scrollY, geometry.rowsHeight);
  if (row != null) {
    final c = row.column!;
    switch (row.type) {
      case LabRowType.sections:
        final s = data.sectionAt(time);
        if (s != null) {
          lines.add(
            LabTipLine(
              '${s.label}  ${s.tier.name}  '
              '${_clock(s.start)}–${_clock(s.end)}',
            ),
          );
        }
      case LabRowType.marks:
        final cy =
            layout.contentY(viewY, geometry.scrollY, geometry.rowsHeight) -
            row.y;
        final part = row.partAt(cy);
        final plan = row.isPlanPart(part);
        final marks = plan ? data.planMarks : data.actualMarks;
        final near = labNearestMarks(
          marks,
          time,
          _hoverReach / geometry.plotWidth * geometry.view.span,
        );
        lines.add(LabTipLine(plan ? 'Plan' : 'Actual', dim: true));
        if (near.isEmpty) {
          lines.add(const LabTipLine('No mark here', dim: true));
        } else {
          for (final m in near) {
            lines.add(
              LabTipLine(labDescribeMark(m, onCut: labMomentOnCut(marks, m))),
            );
          }
        }
      case LabRowType.parameter:
        lines.add(LabTipLine(c.name, bold: true));
        double at(Map<String, Float32List> series) {
          final s = series[c.id];
          if (s == null || s.isEmpty) return double.nan;
          // A series that ends before the song does has no value past its end.
          final i = math.max(LabGeometry.bucketOf(time), 0);
          return i < s.length ? s[i] : double.nan;
        }

        // A line or steps lane tells only the parts that are on; a plan that
        // is missing is told as it is.
        final showPlan =
            !row.isLane || row.showPlan || c.plan != LabPlanAvailability.yes;
        final showActual = !row.isLane || row.showActual;
        switch (showPlan ? c.plan : null) {
          case null:
            break;
          case LabPlanAvailability.yes:
            lines.add(
              LabTipLine(labFormatValue(c, at(data.planSeries)), key: 'Plan'),
            );
          case LabPlanAvailability.no:
            lines.add(
              const LabTipLine('cannot be predicted', key: 'Plan', dim: true),
            );
        }
        if (c.hasActual && showActual) {
          lines.add(
            LabTipLine(labFormatValue(c, at(data.actualSeries)), key: 'Actual'),
          );
        }
      case LabRowType.group:
        break;
    }
  }
  return LabTip(_clock(time), lines);
}

/// The label itself, placed next to the pointer at [anchor] (in the overlay's
/// coordinates), kept inside the overlay. It looks as fluent's tooltips do,
/// from the theme's [TooltipTheme]: their box, padding and text. Only its
/// place is its own, as it follows the pointer over the canvas.
class LabTipLabel extends StatelessWidget {
  const LabTipLabel({super.key, required this.tip, required this.anchor});

  final LabTip tip;
  final Offset anchor;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final style = TooltipTheme.of(context);
    final res = theme.resources;
    // As a tooltip's text, over the body text it sits in; numbers keep
    // their width as they change.
    final text = theme.typography.body!
        .merge(style.textStyle)
        .copyWith(fontFeatures: const [FontFeature.tabularFigures()]);
    final dim = res.textFillColorSecondary;
    final faint = res.textFillColorTertiary;

    return IgnorePointer(
      child: CustomSingleChildLayout(
        delegate: _TipLayout(anchor),
        child: Container(
          decoration: style.decoration,
          padding: style.padding,
          child: DefaultTextStyle(
            style: text,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(tip.time, style: TextStyle(color: dim)),
                for (final line in tip.lines)
                  if (line.key == null)
                    Text(
                      line.text,
                      style: TextStyle(
                        fontWeight: line.bold ? FontWeight.w600 : null,
                        color: line.dim ? faint : null,
                      ),
                    )
                  else
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        SizedBox(
                          width: 48,
                          child: Text(line.key!, style: TextStyle(color: dim)),
                        ),
                        Text(
                          line.text,
                          style: TextStyle(color: line.dim ? faint : null),
                        ),
                      ],
                    ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

// Puts the label below and right of the pointer, and flips it to the other
// side where it would leave the overlay.
class _TipLayout extends SingleChildLayoutDelegate {
  _TipLayout(this.anchor);

  final Offset anchor;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      constraints.loosen();

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    var x = anchor.dx + 14;
    var y = anchor.dy + 14;
    if (x + childSize.width > size.width - 4) {
      x = anchor.dx - childSize.width - 14;
    }
    if (y + childSize.height > size.height - 4) {
      y = anchor.dy - childSize.height - 10;
    }
    return Offset(math.max(4, x), math.max(4, y));
  }

  @override
  bool shouldRelayout(_TipLayout old) => old.anchor != anchor;
}
