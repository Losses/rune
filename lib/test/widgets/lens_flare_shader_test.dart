import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

const _bounds = Rect.fromLTWH(0, 0, 3, 3);

Future<ui.Image> _rasterize(void Function(Canvas) draw) async {
  final recorder = ui.PictureRecorder();
  draw(Canvas(recorder));
  final picture = recorder.endRecording();
  try {
    return await picture.toImage(3, 3);
  } finally {
    picture.dispose();
  }
}

Future<List<int>> _centerPixel(
  ui.FragmentShader shader,
  ui.Image child,
  double flareOpacity, {
  int paintAlpha = 255,
}) async {
  shader
    ..setImageSampler(0, child)
    ..setFloat(0, _bounds.width)
    ..setFloat(1, _bounds.height)
    ..setFloat(2, flareOpacity)
    ..setFloat(3, 0)
    ..setFloat(4, _bounds.center.dx)
    ..setFloat(5, _bounds.center.dy);
  final image = await _rasterize(
    (canvas) => canvas.drawRect(
      _bounds,
      Paint()
        ..shader = shader
        ..color = Color.fromARGB(paintAlpha, 0, 0, 0)
        ..blendMode = BlendMode.src,
    ),
  );
  try {
    final bytes = (await image.toByteData())!;
    return bytes.buffer.asUint8List(4 * 4, 4);
  } finally {
    image.dispose();
  }
}

void main() {
  testWidgets('centered branding flare draws normalized premultiplied colors', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final program = await ui.FragmentProgram.fromAsset(
        'lib/shaders/lens_flare.frag',
      );
      final shader = program.fragmentShader();
      final transparent = await _rasterize((_) {});
      final opaque = await _rasterize(
        (canvas) => canvas.drawColor(const Color(0xFF102030), BlendMode.src),
      );
      try {
        for (final (child, original) in [
          (transparent, [0, 0, 0, 0]),
          (opaque, [16, 32, 48, 255]),
        ]) {
          expect(await _centerPixel(shader, child, 0), original);
          for (final flareOpacity in [1.0, 0.25]) {
            // Modulate before 8-bit readback so shader values above 1 can
            // still exceed the maximum expected channel value of 128.
            final pixel = await _centerPixel(
              shader,
              child,
              flareOpacity,
              paintAlpha: 128,
            );
            final alpha = pixel[3];
            final reason = 'child RGBA $original, flare $flareOpacity: $pixel';
            expect(alpha, inInclusiveRange(1, 128), reason: reason);
            for (final channel in pixel.take(3)) {
              expect(channel, inInclusiveRange(0, alpha), reason: reason);
            }
            expect(pixel[0], greaterThan(original[0] ~/ 2), reason: reason);
            if (child == opaque || flareOpacity == 1) {
              expect(alpha, 128, reason: reason);
            } else {
              // The center alpha is about 1.507 before flare and paint opacity.
              expect(alpha, closeTo(48, 1), reason: reason);
            }
          }
        }
      } finally {
        shader.dispose();
        transparent.dispose();
        opaque.dispose();
      }
    });
  });
}
