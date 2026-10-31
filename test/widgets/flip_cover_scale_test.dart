import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/tile/fast_flip_cover_grid.dart';
import 'package:rune/widgets/tile/utils/flip_grid_painter.dart';
import 'package:rune/widgets/tile/utils/image_memory_manager.dart';
import 'package:rune/widgets/tile/utils/image_proxy.dart';

class _ImageProxy extends ImageProxy {
  _ImageProxy() : super(manager: ImageMemoryManager());
  final requests = <int, Completer<ui.Image>>{};
  @override
  ui.Image? getCachedImage(String path, int size) => null;
  @override
  Future<ui.Image> requestImage(String path, int size) =>
      (requests[size] = Completer<ui.Image>()).future;
  @override
  void dispose() {}
}

Future<ui.Image> _image() async {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  canvas.drawRect(const Rect.fromLTWH(0, 0, 2, 2), Paint()..color = Colors.red);
  return recorder.endRecording().toImage(2, 2);
}

void main() {
  testWidgets('reloads for scale and size and ignores stale completions', (
    tester,
  ) async {
    final proxy = _ImageProxy();
    Widget grid(double dpr, int size) => FluentApp(
      home: MediaQuery(
        data: MediaQueryData(size: const Size(400, 400), devicePixelRatio: dpr),
        child: Center(
          child: SizedBox(
            width: 100,
            height: 100,
            child: FastFlipCoverGrid(
              paths: const ['cover'],
              size: size,
              name: 'test',
              colors: [Colors.red],
              imageProxy: proxy,
            ),
          ),
        ),
      ),
    );
    FlipGridPainter painter() =>
        tester
                .widget<CustomPaint>(
                  find.descendant(
                    of: find.byType(FastFlipCoverGrid),
                    matching: find.byType(CustomPaint),
                  ),
                )
                .painter!
            as FlipGridPainter;
    await tester.pumpWidget(grid(1, 100));
    expect(proxy.requests.keys, contains(100));
    await tester.pumpWidget(grid(2, 100));
    expect(proxy.requests.keys, contains(200));
    final high = await _image();
    final stale = await _image();
    proxy.requests[200]!.complete(high);
    await tester.pump();
    expect(painter().images.single, same(high));
    proxy.requests[100]!.complete(stale);
    await tester.pump();
    expect(painter().images.single, same(high));
    await tester.pumpWidget(grid(2, 150));
    expect(proxy.requests.keys, contains(300));
    final resized = await _image();
    proxy.requests[300]!.complete(resized);
    await tester.pump();
    expect(painter().images.single, same(resized));
    await tester.pumpWidget(const SizedBox());
    high.dispose();
    stale.dispose();
    resized.dispose();
  });
}
