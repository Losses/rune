import '../../../screens/settings_test/settings_test.dart';
import 'package:fluent_ui/fluent_ui.dart';

class TrackNumberPainter extends CustomPainter {
  final int? number1;
  final int number2;
  final Color color;

  TrackNumberPainter({
    this.number1,
    required this.number2,
    required this.color,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (number1 != null) {
      // The first number
      final path1 = Path()
        ..moveTo(0, 0)
        ..lineTo(size.width, 0)
        ..lineTo(0, size.height)
        ..close();

      canvas.save();
      canvas.clipPath(path1);

      final textPainter1 = TextPainter(
        text: TextSpan(
          text: number1.toString().padLeft(2, '0'),
          style: TextStyle(
            fontSize: size.width * fontSizeFactor,
            fontWeight: FontWeight.bold,
            color: color.withAlpha(180),
          ),
        ),
        textDirection: TextDirection.ltr,
      );

      textPainter1.layout(
        maxWidth: size.width * 0.8,
      );

      textPainter1.paint(
        canvas,
        Offset(
          size.width * 0.1,
          size.height * 0.1,
        ),
      );

      canvas.restore();

      // The second number
      final path2 = Path()
        ..moveTo(size.width, size.height)
        ..lineTo(0, size.height)
        ..lineTo(size.width, 0)
        ..close();

      canvas.save();
      canvas.clipPath(path2);

      final textPainter2 = TextPainter(
        text: TextSpan(
          text: number2.toString().padLeft(2, '0'),
          style: TextStyle(
            fontSize: size.width * fontSizeFactor,
            fontWeight: FontWeight.bold,
            color: color,
          ),
        ),
        textDirection: TextDirection.ltr,
      );

      textPainter2.layout(
        maxWidth: size.width * 0.8,
      );

      textPainter2.paint(
        canvas,
        Offset(
          size.width - textPainter2.width - size.width * 0.1,
          size.height - textPainter2.height - size.height * 0.1,
        ),
      );

      canvas.restore();

      // Draw the diagonal line with gradient
      final paint = Paint()
        ..shader = LinearGradient(
          colors: [
            Colors.transparent,
            color,
            Colors.transparent,
          ],
          stops: [0.0, 0.5, 1.0],
        ).createShader(
          Rect.fromPoints(
            Offset(size.width, 0),
            Offset(0, size.height),
          ),
        )
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.0; // Adjust the stroke width as needed

      canvas.drawLine(
        Offset(size.width, 0),
        Offset(0, size.height),
        paint,
      );
    } else {
      // Center the second number
      final textPainter2 = TextPainter(
        text: TextSpan(
          text: number2.toString().padLeft(2, '0'),
          style: TextStyle(
            fontSize: size.width * fontSizeFactor,
            fontWeight: FontWeight.bold,
          ),
        ),
        textDirection: TextDirection.ltr,
      );

      textPainter2.layout(
        maxWidth: size.width * 0.8,
      );

      textPainter2.paint(
        canvas,
        Offset(
          (size.width - textPainter2.width) / 2,
          (size.height - textPainter2.height) / 2,
        ),
      );
    }
  }

  @override
  bool shouldRepaint(TrackNumberPainter oldDelegate) {
    return oldDelegate.number1 != number1 || oldDelegate.number2 != number2;
  }
}
