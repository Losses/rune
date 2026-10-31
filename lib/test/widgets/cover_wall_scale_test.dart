import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/cover_wall_background/cover_wall_background.dart';
import 'package:rune/widgets/cover_wall_background/utils/cover_wall_background_painter.dart';

Future<ui.Image> _image() async {
  final recorder = ui.PictureRecorder();
  Canvas(
    recorder,
  ).drawRect(const Rect.fromLTWH(0, 0, 2, 2), Paint()..color = Colors.red);
  return recorder.endRecording().toImage(2, 2);
}

void main() {
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
