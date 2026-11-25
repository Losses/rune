import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/visualizer_lab.dart';

/// The duration the lab mode's changes take, as in the router's animation.
const labModeDuration = Duration(milliseconds: 300);

/// The curve the lab mode's changes take, as in the router's animation.
const labModeCurve = Curves.easeOutQuint;

/// Builds with a value that goes from 0 to 1 when the lab mode comes on and
/// back when it goes off, which [enterCurve] and [exitCurve] shape.
///
/// It does not animate on the first build, so a page that is already in the lab
/// mode shows it at once.
class LabModeBuilder extends StatelessWidget {
  const LabModeBuilder({
    super.key,
    required this.builder,
    this.child,
    this.enterDuration = labModeDuration,
    this.exitDuration = labModeDuration,
    this.enterCurve = labModeCurve,
    this.exitCurve = labModeCurve,
  });

  final Widget Function(
    BuildContext context,
    double value,
    bool on,
    Widget? child,
  )
  builder;
  final Widget? child;
  final Duration enterDuration;
  final Duration exitDuration;
  final Curve enterCurve;
  final Curve exitCurve;

  @override
  Widget build(BuildContext context) {
    final on = watchVisualizerLabMode(context);
    return TweenAnimationBuilder<double>(
      tween: Tween<double>(end: on ? 1 : 0),
      duration: on ? enterDuration : exitDuration,
      curve: on ? enterCurve : exitCurve,
      child: child,
      builder: (context, value, child) => builder(context, value, on, child),
    );
  }
}
