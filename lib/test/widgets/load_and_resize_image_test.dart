import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/utils/load_and_resize_image.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'decodes to cover the requested square and returns a usable image',
    () async {
      final dir = await Directory.systemTemp.createTemp('rune-resize-test-');
      try {
        final recorder = ui.PictureRecorder();
        ui.Canvas(recorder).drawRect(
          const ui.Rect.fromLTWH(0, 0, 40, 20),
          ui.Paint()..color = const ui.Color(0xffff0000),
        );
        final source = await recorder.endRecording().toImage(40, 20);
        final bytes = await source.toByteData(format: ui.ImageByteFormat.png);
        source.dispose();
        final file = File('${dir.path}/cover.png');
        await file.writeAsBytes(bytes!.buffer.asUint8List());
        final result = await loadAndResizeImage(file.path, 10);
        expect(result.width, 20);
        expect(result.height, 10);
        expect(await result.toByteData(), isNotNull);
        result.dispose();
        await file.writeAsBytes([1, 2, 3]);
        await expectLater(
          loadAndResizeImage(file.path, 10),
          throwsA(isA<Exception>()),
        );
      } finally {
        await dir.delete(recursive: true);
      }
    },
  );
}
