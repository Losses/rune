import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

class _SplitBackground extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(
      Rect.fromLTWH(0, 0, size.width / 2, size.height),
      Paint()..color = const Color(0xffff0000),
    );
    canvas.drawRect(
      Rect.fromLTWH(size.width / 2, 0, size.width / 2, size.height),
      Paint()..color = const Color(0xff0000ff),
    );
  }

  @override
  bool shouldRepaint(_SplitBackground oldDelegate) => false;
}

void main() {
  testWidgets('acrylic blurs the backdrop without tinting outside its bounds', (
    tester,
  ) async {
    final key = GlobalKey();
    await tester.pumpWidget(
      FluentApp(
        home: Center(
          child: RepaintBoundary(
            key: key,
            child: SizedBox(
              width: 300,
              height: 200,
              child: Stack(
                children: [
                  Positioned.fill(
                    child: CustomPaint(painter: _SplitBackground()),
                  ),
                  const Positioned(
                    left: 50,
                    top: 50,
                    width: 200,
                    height: 100,
                    child: Acrylic(child: SizedBox.expand()),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final box =
        key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    final pixels = await tester.runAsync(() async {
      final image = await box.toImage();
      try {
        return await image.toByteData(format: ui.ImageByteFormat.rawRgba);
      } finally {
        image.dispose();
      }
    });
    int channel(int x, int y, int c) => pixels!.getUint8((y * 300 + x) * 4 + c);
    expect(channel(10, 100, 0), 255);
    expect(channel(10, 100, 2), 0);
    expect(channel(290, 100, 0), 0);
    expect(channel(290, 100, 2), 255);
    // The red/blue transition must spread into both halves of the surface.
    // Removing the blur would leave each half a uniform color.
    expect(channel(140, 100, 2), greaterThan(channel(70, 100, 2)));
    expect(channel(160, 100, 0), greaterThan(channel(230, 100, 0)));
    expect(tester.takeException(), isNull);
  });
}
