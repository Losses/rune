import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/visualizer/lab/lab_segmented.dart';

const _width = 240;
const _height = 30;

void main() {
  // What a choice of three paints, the chosen one [value], as RGBA bytes.
  Future<ByteData> paint(WidgetTester tester, int value) async {
    final key = GlobalKey();
    await tester.pumpWidget(
      FluentApp(
        darkTheme: FluentThemeData(brightness: Brightness.dark),
        themeMode: ThemeMode.dark,
        color: const Color(0x00000000),
        home: Align(
          alignment: Alignment.topLeft,
          child: RepaintBoundary(
            key: key,
            child: SizedBox(
              width: _width.toDouble(),
              child: LabSegmented<int>(
                value: value,
                height: _height.toDouble(),
                items: const [(0, 'A'), (1, 'B'), (2, 'C')],
                onChanged: (_) {},
              ),
            ),
          ),
        ),
      ),
    );
    final boundary =
        key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
    final bytes = await tester.runAsync(() async {
      final image = await boundary.toImage();
      return image.toByteData();
    });
    return bytes!;
  }

  int alphaAt(ByteData bytes, int x, int y) =>
      bytes.getUint8((y * _width + x) * 4 + 3);

  testWidgets('a chosen end rounds with the frame: its fill stays inside '
      'the border\'s curve', (tester) async {
    // In a corner, just inside the border's curve: the border alone, faint,
    // where a fill clipped to the outer edge would cover it.
    var bytes = await paint(tester, 0);
    expect(alphaAt(bytes, 20, _height ~/ 2), 255, reason: 'the fill');
    expect(alphaAt(bytes, 1, 1), lessThan(128));
    expect(alphaAt(bytes, 1, _height - 2), lessThan(128));

    bytes = await paint(tester, 2);
    expect(alphaAt(bytes, _width - 20, _height ~/ 2), 255, reason: 'the fill');
    expect(alphaAt(bytes, _width - 2, 1), lessThan(128));
    expect(alphaAt(bytes, _width - 2, _height - 2), lessThan(128));
  });
}
