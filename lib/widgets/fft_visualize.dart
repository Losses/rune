import 'dart:ui' as ui;
import 'package:fluent_ui/fluent_ui.dart';

import '../messages/playback.pb.dart';

class FFTVisualize extends StatelessWidget {
  const FFTVisualize({super.key});

  final radius = 12.0;

  @override
  Widget build(BuildContext context) {
    final color = FluentTheme.of(context).accentColor;

    return StreamBuilder(
      stream: RealtimeFFT.rustSignalStream, // GENERATED
      builder: (context, snapshot) {
        final rustSignal = snapshot.data;
        if (rustSignal == null) {
          return const Text("Nothing received yet");
        }
        final fftValue = rustSignal.message.value;
        return LayoutBuilder(builder: (context, constraints) {
          double parentHeight = constraints.maxHeight;

          return OverflowBox(
              maxHeight: parentHeight * 2,
              alignment: Alignment.topCenter,
              child: SizedBox(
                height: parentHeight * 2,
                child: Opacity(
                    opacity: 0.87,
                    child: ImageFiltered(
                      imageFilter:
                          ui.ImageFilter.blur(sigmaX: radius, sigmaY: radius),
                      child: CustomPaint(
                        painter: FFTPainter(fftValue, color),
                      ),
                    )),
              ));
        });
      },
    );
  }
}

class FFTPainter extends CustomPainter {
  final List<double> fftValues;
  final Color color;

  FFTPainter(this.fftValues, this.color);

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = color
      ..strokeWidth = size.width / fftValues.length;

    final midY = size.height / 2;

    for (int i = 0; i < fftValues.length; i++) {
      final x = i * (size.width / fftValues.length);
      final y = fftValues[i] * size.height / 2;
      canvas.drawLine(Offset(x, midY - y), Offset(x, midY + y), paint);
    }
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) {
    return true;
  }
}
