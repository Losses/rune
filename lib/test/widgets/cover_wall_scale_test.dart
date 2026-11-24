import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/cover_wall_background/cover_wall_background.dart';
import 'package:rune/widgets/cover_wall_background/utils/cover_wall_background_painter.dart';

Future<ui.Image> _image([Color color = const Color(0xFFFF0000)]) async {
  final recorder = ui.PictureRecorder();
  Canvas(
    recorder,
  ).drawRect(const Rect.fromLTWH(0, 0, 2, 2), Paint()..color = color);
  final picture = recorder.endRecording();
  try {
    return await picture.toImage(2, 2);
  } finally {
    picture.dispose();
  }
}

void main() {
  for (final failedFirst in [true, false]) {
    final position = failedFirst ? 'first' : 'middle';
    testWidgets('cover wall paints after a failed $position image', (
      tester,
    ) async {
      const wallSize = Size(640, 480);
      final boundary = GlobalKey();
      final paths = [if (!failedFirst) 'earlier', 'missing', 'later'];
      final loads = {for (final path in paths) path: Completer<ui.Image>()};
      final earlier = failedFirst ? null : await _image();
      final later = await _image(const Color(0xFF00FF00));
      await tester.pumpWidget(
        FluentApp(
          home: Center(
            child: SizedBox.fromSize(
              size: wallSize,
              child: RepaintBoundary(
                key: boundary,
                child: CoverWallBackground(
                  seed: 1,
                  gap: 0,
                  loadPaths: () async => paths,
                  loadImage: (path, _) => loads[path]!.future,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      loads['missing']!.completeError(StateError('cover unavailable'));
      if (earlier != null) loads['earlier']!.complete(earlier);
      await tester.pump();
      loads['later']!.complete(later);
      await tester.pump();
      await tester.pump();

      final painter = tester
          .widgetList<CustomPaint>(
            find.descendant(
              of: find.byType(CoverWallBackground),
              matching: find.byType(CustomPaint),
            ),
          )
          .map((widget) => widget.painter)
          .whereType<CoverWallBackgroundPainter>()
          .single;
      expect(painter.images.last, same(later));
      final render =
          boundary.currentContext!.findRenderObject() as RenderRepaintBoundary;
      final pixels = (await tester.runAsync(() async {
        final image = await render.toImage();
        try {
          return await image.toByteData(format: ui.ImageByteFormat.rawRgba);
        } finally {
          image.dispose();
        }
      }))!;

      void expectTile(String path, int rgba) {
        final step = painter.gridSize;
        final tile = painter.grid[paths.indexOf(path)].firstWhere(
          (tile) =>
              tile.col * step + step ~/ 2 < wallSize.width &&
              tile.row * step + step ~/ 2 < wallSize.height,
        );
        final x = tile.col * step + step ~/ 2;
        final y = tile.row * step + step ~/ 2;
        expect(
          pixels.getUint32(4 * (y * wallSize.width.toInt() + x)),
          rgba,
          reason: '$path cover tile',
        );
      }

      if (!failedFirst) expectTile('earlier', 0xFF0000FF);
      expectTile('missing', 0x00000000);
      expectTile('later', 0x00FF00FF);
    });
  }

  testWidgets(
    'cover wall reloads on DPR and layout changes and ignores stale images',
    (tester) async {
      tester.view.physicalSize = const Size(5000, 1000);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final requests = <int, Completer<ui.Image>>{};
      Future<ui.Image> load(String path, int size) =>
          (requests[size] = Completer<ui.Image>()).future;
      Widget wall(double dpr, double width) => FluentApp(
        home: MediaQuery(
          data: MediaQueryData(size: Size(width, 400), devicePixelRatio: dpr),
          child: Center(
            child: SizedBox(
              width: width,
              height: 200,
              child: CoverWallBackground(
                seed: 1,
                gap: 0,
                loadPaths: () async => ['cover'],
                loadImage: load,
              ),
            ),
          ),
        ),
      );
      CoverWallBackgroundPainter painter() => tester
          .widgetList<CustomPaint>(
            find.descendant(
              of: find.byType(CoverWallBackground),
              matching: find.byType(CustomPaint),
            ),
          )
          .map((w) => w.painter)
          .whereType<CoverWallBackgroundPainter>()
          .single;
      await tester.pumpWidget(wall(1, 400));
      await tester.pump();
      await tester.pump();
      final lowKey = requests.keys.single;
      await tester.pumpWidget(wall(2, 400));
      await tester.pump();
      final highKey = requests.keys.singleWhere((size) => size != lowKey);
      final low = await _image();
      final high = await _image();
      requests[highKey]!.complete(high);
      await tester.pump();
      expect(painter().images.single, same(high));
      requests[lowKey]!.complete(low);
      await tester.pump();
      expect(painter().images.single, same(high));
      await tester.pumpWidget(wall(2, 4000));
      await tester.pump();
      final resizedKey = requests.keys.singleWhere(
        (size) => size != lowKey && size != highKey,
      );
      final resized = await _image();
      requests[resizedKey]!.complete(resized);
      await tester.pump();
      expect(painter().images.single, same(resized));
      await tester.pumpWidget(const SizedBox());
    },
  );
}
