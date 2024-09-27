import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import './fancy_cover_config.dart';

class FancyCoverPainter extends CustomPainter {
  final List<String> texts;
  final List<FancyCoverConfig> configs;
  final Size canvasSize;
  final Color color;

  FancyCoverPainter({
    required this.texts,
    required this.configs,
    required this.canvasSize,
    required this.color,
  }) : assert(texts.length == configs.length);

  @override
  void paint(Canvas canvas, Size size) {
    for (int i = 0; i < texts.length; i++) {
      _drawText(canvas, size, texts[i], configs[i]);
    }
  }

  void _drawText(
      Canvas canvas, Size size, String text, FancyCoverConfig config) {
    final scaledFontSize = config.fontSize * size.width / canvasSize.width;
    final scaledTextBoxWidth = config.textBoxWidth * canvasSize.width;

    final textStyle = TextStyle(
      color: color,
      fontSize: scaledFontSize,
      fontWeight: config.fontWeight,
      height: 1,
    );

    final textSpan = TextSpan(
      text: config.toUpperCase ? text.toUpperCase() : text,
      style: textStyle,
    );

    final textPainter = TextPainter(
      text: textSpan,
      textDirection: TextDirection.ltr,
      textAlign: config.textAlign,
      textHeightBehavior:
          const TextHeightBehavior(applyHeightToFirstAscent: false),
    );

    textPainter.layout(
      minWidth: 0,
      maxWidth: scaledTextBoxWidth,
    );

    final xCenter = size.width * config.position.dx -
        textPainter.width / 2 -
        textPainter.width * config.transformOrigin.dx;
    final yCenter = size.height * config.position.dy -
        textPainter.height / 2 -
        textPainter.height * config.transformOrigin.dy;

    canvas.save();
    canvas.rotate(config.rotation * pi / 180);
    canvas.translate(xCenter, yCenter);
    canvas.translate(
      config.transformOrigin.dx * textPainter.width,
      config.transformOrigin.dy * textPainter.height,
    );
    canvas.translate(
      -config.transformOrigin.dx * textPainter.width,
      -config.transformOrigin.dy * textPainter.height,
    );

    textPainter.paint(canvas, Offset.zero);
    canvas.restore();
  }

  @override
  bool shouldRepaint(FancyCoverPainter oldDelegate) =>
      texts != oldDelegate.texts || configs != oldDelegate.configs;
}
