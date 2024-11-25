import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

import 'reveal_config.dart';

class RevealEffectPainter extends CustomPainter {
  final Offset? mousePosition;
  final bool mousePressed;
  final bool mouseReleased;
  final Offset? mouseUpPosition;
  final double mouseDownAnimateFrame;
  final RevealConfig config;

  RevealEffectPainter({
    this.mousePosition,
    this.mousePressed = false,
    this.mouseReleased = false,
    this.mouseUpPosition,
    this.mouseDownAnimateFrame = 0,
    this.config = const RevealConfig(),
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (mousePosition == null && !mousePressed) return;

    final path = Path();
    final rrect = config.borderRadius
        .resolve(TextDirection.ltr)
        .toRRect(Offset.zero & size);
    path.addRRect(rrect);

    if (config.hoverLight &&
        mousePosition != null &&
        mousePosition!.dx >= 0 &&
        mousePosition!.dx <= size.width &&
        mousePosition!.dy >= 0 &&
        mousePosition!.dy <= size.height) {
      _drawHoverLight(canvas, size, path);
    }

    if (config.borderWidth > 0 && config.diffuse && mousePosition != null) {
      _drawBorderLight(canvas, size, path);
    }

    if (config.pressAnimation && mousePressed) {
      _drawPressAnimation(canvas, size, path);
    }
  }

  void _drawHoverLight(Canvas canvas, Size size, Path path) {
    final radius = size.shortestSide * config.hoverLightFillRadius;
    final gradient = ui.Gradient.radial(
      mousePosition!,
      radius,
      [
        config.hoverLightColor.withOpacity(config.opacity * 0.5),
        config.hoverLightColor.withOpacity(0),
      ],
    );

    canvas.save();
    canvas.clipPath(path);
    canvas.drawPaint(Paint()..shader = gradient);
    canvas.restore();
  }

  void _drawBorderLight(Canvas canvas, Size size, Path path) {
    final radius = size.shortestSide * config.borderFillRadius;
    final gradient = ui.Gradient.radial(
      mousePosition!,
      radius,
      [
        config.borderColor.withOpacity(config.opacity),
        config.borderColor.withOpacity(0),
      ],
    );

    final borderPath = Path();
    final rrect = config.borderRadius
        .resolve(TextDirection.ltr)
        .toRRect(Offset.zero & size);
    final innerRRect = config.borderRadius.resolve(TextDirection.ltr).toRRect(
        Offset(config.borderWidth, config.borderWidth) &
            Size(size.width - config.borderWidth * 2,
                size.height - config.borderWidth * 2));
    borderPath.addRRect(rrect);
    borderPath.addRRect(innerRRect);
    borderPath.fillType = PathFillType.evenOdd;

    canvas.save();
    canvas.clipPath(borderPath);
    canvas.drawPaint(Paint()..shader = gradient);
    canvas.restore();
  }

  void _drawPressAnimation(Canvas canvas, Size size, Path path) {
    final position = mouseReleased ? mouseUpPosition : mousePosition;

    if (position == null) return;

    final radius = config.pressAnimationFillMode == 'constrained'
        ? size.shortestSide
        : size.longestSide;

    final innerAlpha = (0.2 - mouseDownAnimateFrame) * config.opacity;
    final outerAlpha = (0.1 - mouseDownAnimateFrame * 0.07) * config.opacity;
    final outerBorder = (0.1 + mouseDownAnimateFrame * 0.8).clamp(0.0, 1.0);

    final gradient = ui.Gradient.radial(
      position,
      radius,
      [
        config.pressAnimationColor.withOpacity(innerAlpha.clamp(0.0, 1.0)),
        config.pressAnimationColor.withOpacity(outerAlpha.clamp(0.0, 1.0)),
        config.pressAnimationColor.withOpacity(0),
      ],
      [0, outerBorder * 0.55, outerBorder],
    );

    canvas.save();
    canvas.clipPath(path);
    canvas.drawPaint(Paint()..shader = gradient);
    canvas.restore();
  }

  @override
  bool shouldRepaint(RevealEffectPainter oldDelegate) {
    return mousePosition != oldDelegate.mousePosition ||
        mousePressed != oldDelegate.mousePressed ||
        mouseReleased != oldDelegate.mouseReleased ||
        mouseUpPosition != oldDelegate.mouseUpPosition ||
        mouseDownAnimateFrame != oldDelegate.mouseDownAnimateFrame;
  }
}
