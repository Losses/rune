import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/visualizer_lab.dart';
import 'lab_data.dart';

// The progress bar's slider keeps this much of its width free on each side,
// which is where its thumb starts and ends.
const _sliderInset = 10.0;

/// Puts the plan's section boundaries and cuts behind [slider], a progress
/// bar's slider, while the lab is on. The tree keeps its shape whether there is
/// a plan or not, so that a plan coming or going does not cut a drag short, and
/// the slider is not rebuilt as the lab polls. With the lab off it is [slider]
/// itself, so the tree is as it is without the lab.
Widget withLabMarkers(
  BuildContext context, {
  required Widget slider,
  required double duration,
}) {
  final enabled = context.select<VisualizerLabProvider, bool>((p) => p.enabled);
  if (!enabled) return slider;

  final timeline = context.read<VisualizerLabProvider>().timeline;
  final color = FluentTheme.of(context).resources.textFillColorPrimary;

  return Stack(
    alignment: Alignment.center,
    children: [
      Positioned.fill(
        child: IgnorePointer(
          child: ValueListenableBuilder<LabTimelineData>(
            valueListenable: timeline,
            builder: (context, data, _) => CustomPaint(
              painter: LabProgressMarkersPainter(
                sections: data.sections,
                planMarks: data.planMarks,
                // The slider spans the player's duration, which the chart's
                // length may miss by a little.
                duration: duration > 0 ? duration : data.length,
                inset: _sliderInset,
                color: color,
              ),
            ),
          ),
        ),
      ),
      slider,
    ],
  );
}

/// Draws the section boundaries and the planned cuts, thin and faint, for
/// behind a progress bar.
class LabProgressMarkersPainter extends CustomPainter {
  LabProgressMarkersPainter({
    required this.sections,
    required this.planMarks,
    required this.duration,
    required this.inset,
    required this.color,
  });

  final List<LabSection> sections;
  final List<LabMark> planMarks;
  final double duration;
  final double inset;
  final Color color;

  double _x(double time, Size size) {
    final width = size.width - 2 * inset;
    return inset + (duration <= 0 ? 0 : (time / duration).clamp(0, 1)) * width;
  }

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.height / 2;
    final boundary = Paint()
      ..color = color.withAlpha(70)
      ..strokeWidth = 1;
    for (final s in sections) {
      if (s.start <= 0) continue;
      final x = _x(s.start, size);
      canvas.drawLine(Offset(x, mid - 8), Offset(x, mid + 8), boundary);
    }
    final tick = Paint()..color = color.withAlpha(110);
    for (final m in planMarks) {
      if (m.kind != LabMarkKind.cut) continue;
      final x = _x(m.time, size);
      tick.strokeWidth = m.strong ? 2 : 1;
      canvas.drawLine(Offset(x, mid - 4), Offset(x, mid + 4), tick);
    }
  }

  @override
  bool shouldRepaint(LabProgressMarkersPainter old) =>
      !identical(old.sections, sections) ||
      !identical(old.planMarks, planMarks) ||
      old.duration != duration ||
      old.inset != inset ||
      old.color != color;
}
