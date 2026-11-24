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
    if (size.isFinite) return _paint(Size.square(size));
    // An infinite size asks the cover to fill its parent. The parent bounds at
    // least one side where the cover is used; an unbounded side takes the
    // bounded one so the cover stays square.
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = constraints.hasBoundedWidth
            ? constraints.maxWidth
            : constraints.hasBoundedHeight
            ? constraints.maxHeight
            : 0.0;
        final height = constraints.hasBoundedHeight
            ? constraints.maxHeight
            : width;
        return _paint(Size(width, height));
      },
    );
  }

  Widget _paint(Size paintSize) {
    return Container(
      color: background,
      child: CustomPaint(
        size: paintSize,
        painter: FancyCoverPainter(
          texts: texts,
          configs: configs,
          color: foreground,
        ),
      ),
    );
  }
}
