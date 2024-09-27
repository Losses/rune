import 'package:fluent_ui/fluent_ui.dart';

import './fancy_cover_config.dart';
import './fancy_cover_painter.dart';

class FancyCoverImplementation extends StatelessWidget {
  final double size;
  final List<String> texts;
  final List<FancyCoverConfig> configs;
  final Color foreground;
  final Color background;

  const FancyCoverImplementation({
    super.key,
    required this.size,
    required this.texts,
    required this.configs,
    required this.foreground,
    required this.background,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      color: background,
      child: CustomPaint(
        size: Size.square(size),
        painter: FancyCoverPainter(
          texts: texts,
          configs: configs,
          canvasSize: Size.square(size),
          color: foreground,
        ),
      ),
    );
  }
}
