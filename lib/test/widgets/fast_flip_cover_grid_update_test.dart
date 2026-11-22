import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/tile/fast_flip_cover_grid.dart';
import '../../widgets/tile/utils/flip_grid_painter.dart';
import '../../widgets/tile/utils/image_memory_manager.dart';
import '../../widgets/tile/utils/image_proxy.dart';

class _ImageProxy extends ImageProxy {
  _ImageProxy() : super(manager: ImageMemoryManager());
  final requests = <String, Completer<ui.Image>>{};
  @override
  ui.Image? getCachedImage(String path, int size) => null;
  @override
  Future<ui.Image> requestImage(String path, int size) =>
      (requests[path] = Completer<ui.Image>()).future;
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
  group('the cover a cell turns to', () {
    test('is none while every cover is shown', () {
      const covers = ['a', 'b', 'c', 'd'];
      for (final r in [0.0, 0.3, 0.99]) {
        expect(freeCover(covers, covers.toSet(), r), isNull);
      }
    });

    test('is never one that is shown or about to be', () {
      const covers = ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'];
      const inUse = {'a', 'c', 'e', 'g', 'h'};
      final picked = {
        for (var i = 0; i < 100; i += 1) freeCover(covers, inUse, i / 100),
      };
      expect(picked, {'b', 'd', 'f'});
    });
  });

  testWidgets(
    'a card that a reload gives another collection shows its covers',
    (tester) async {
      final proxy = _ImageProxy();
      // The card a list kept on screen while it reloads passes new paths to.
      Widget grid(List<String> paths) => FluentApp(
        home: Center(
          child: SizedBox(
            width: 100,
            height: 100,
            child: FastFlipCoverGrid(
              paths: paths,
              size: 100,
              name: 'card',
              colors: [Colors.red],
              imageProxy: proxy,
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

      await tester.pumpWidget(grid(const ['a']));
      final a = await _image();
      proxy.requests['a']!.complete(a);
      await tester.pump();
      expect(painter().images.single, same(a));

      await tester.pumpWidget(grid(const ['b']));
      expect(proxy.requests.keys, contains('b'));
      expect(painter().images.single, isNull);
      final b = await _image();
      proxy.requests['b']!.complete(b);
      await tester.pump();
      expect(painter().images.single, same(b));

      // Covers enough to flip start the flip timer, and one cover stops it.
      await tester.pumpWidget(grid(const ['c', 'd', 'e', 'f']));
      expect(painter().gridCount, 2);
      await tester.pump(const Duration(seconds: 10));
      await tester.pumpWidget(grid(const ['g']));
      expect(painter().gridCount, 1);
      await tester.pumpWidget(const SizedBox());
      a.dispose();
      b.dispose();
    },
  );

  test('a grid painted with another cell count repaints', () {
    FlipGridPainter painter(int gridCount) => FlipGridPainter(
      const [],
      rotates: const [0],
      fallbackColors: const [Color(0xFFFF0000)],
      gridCount: gridCount,
    );
    expect(painter(3).shouldRepaint(painter(2)), isTrue);
  });
}
