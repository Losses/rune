import 'package:fluent_ui/fluent_ui.dart';

class FlipTextPositions {
  final BuildContext context;
  final Offset position;

  FlipTextPositions({
    required this.context,
    required this.position,
  });
}

class FlipTextStyles {
  double scale;
  double? fontWeight;
  Color? color;
  double? alpha;

  FlipTextStyles({
    required this.scale,
    required this.fontWeight,
    required this.color,
    required this.alpha,
  });
}
