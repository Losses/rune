import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../screens/cover_wall/widgets/cover_wall_layout.dart';
import '../../widgets/cover_wall_background/gradient_container.dart';
import '../../widgets/cover_wall_background/gradient_noise.dart';

const _width = 96, _height = 72;
const _color = Color(0xFF0078D4), _color2 = Color(0xFF06121F);

// Pixels the gradient shader drew before it took its noise gradients and hue
// shifts from the app (it hashed and turned them for every pixel), with the
// cover wall's parameters, at 6 x 5 points of the drawing below: in the
// order of the times, pointers and modes of [_drawings], RGBA.
const _drawn = [
  [
    0x000000ff,
    0x0b1113ff,
    0x0b2539ff,
    0x02182aff,
    0x02223aff,
    0x010e19ff,
    0x142127ff,
    0x142b3aff,
    0x05243dff,
    0x032c4bff,
    0x04355bff,
    0x021422ff,
    0x134a74ff,
    0x093b62ff,
    0x054a7eff,
    0x05538fff,
    0x064f87ff,
    0x073e68ff,
    0x074371ff,
    0x054a7fff,
    0x075ea1ff,
    0x085a99ff,
    0x0662a8ff,
    0x04406eff,
    0x01080eff,
    0x02131fff,
    0x041929ff,
    0x06243bff,
    0x042c49ff,
    0x01090eff,
  ],
  [
    0x0078d4ff,
    0x0078d4ec,
    0x0078d4c6,
    0x0078d4d0,
    0x0078d4bc,
    0x0078d4e3,
    0x0167b7d8,
    0x0168b8c5,
    0x0070c6bc,
    0x0074cda8,
    0x0072ca95,
    0x0076d0d9,
    0x03467b89,
    0x03417393,
    0x02508d6d,
    0x025ea65a,
    0x015fa864,
    0x0164b08a,
    0x042d4f7f,
    0x0616266c,
    0x042c4d46,
    0x024f8b50,
    0x025ba13d,
    0x02599e80,
    0x033e6eee,
    0x052541db,
    0x04365ed2,
    0x034b84bf,
    0x025596ac,
    0x025a9eef,
  ],
  [
    0x0078d4ff,
    0x0078d4ec,
    0x0077d3c6,
    0x0078d4d0,
    0x0078d4bc,
    0x0078d4e3,
    0x0078d4d8,
    0x0078d4c5,
    0x0078d4bc,
    0x0078d4a8,
    0x0078d495,
    0x0078d4d9,
    0x0078d489,
    0x0078d493,
    0x0078d46d,
    0x0078d45a,
    0x0078d464,
    0x0078d48a,
    0x0078d47f,
    0x0078d46c,
    0x0078d446,
    0x0078d450,
    0x0078d43d,
    0x0078d480,
    0x0078d4ee,
    0x0078d4db,
    0x0078d4d2,
    0x0078d4bf,
    0x0078d4ac,
    0x0078d4ef,
  ],
  [
    0x000000ff,
    0x040c13ff,
    0x0d2639ff,
    0x0d222fff,
    0x0b2a43ff,
    0x000d17ff,
    0x132527ff,
    0x09243aff,
    0x0d2a43ff,
    0x194057ff,
    0x16466aff,
    0x031426ff,
    0x0c436dff,
    0x01335bff,
    0x074b92ff,
    0x1561a5ff,
    0x155c9bff,
    0x0c4275ff,
    0x003a68ff,
    0x004176ff,
    0x005398ff,
    0x0355a0ff,
    0x0f69c2ff,
    0x0d487fff,
    0x00070dff,
    0x00101dff,
    0x011626ff,
    0x022037ff,
    0x052b53ff,
    0x020910ff,
  ],
  [
    0x000000ff,
    0x00080fff,
    0x011c30ff,
    0x001627ff,
    0x001d35ff,
    0x000c16ff,
    0x001323ff,
    0x00192fff,
    0x001d35ff,
    0x002544ff,
    0x002d54ff,
    0x00111eff,
    0x013b69ff,
    0x00325bff,
    0x004075ff,
    0x004782ff,
    0x00427aff,
    0x00325cff,
    0x003765ff,
    0x003f74ff,
    0x004f92ff,
    0x004c8bff,
    0x005399ff,
    0x003664ff,
    0x00070dff,
    0x000f1cff,
    0x001424ff,
    0x001d34ff,
    0x002442ff,
    0x00070dff,
  ],
];

// Times out of order, so that the lattice computes its gradients again in
// both directions; the last is past 289, where the noise repeats.
const _drawings = [
  (0.37, Offset.zero, true),
  (123.45, Offset(25, 55), false),
  (37.25, Offset.zero, false),
  (286.5, Offset(60, 20), true),
  (1000.6, Offset(10, 10), true),
];

Future<ui.Image> _covers() {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  for (var i = 0; i < 8; i++) {
    for (var j = 0; j < 6; j++) {
      canvas.drawRect(
        Rect.fromLTWH(i * 12.0, j * 12.0, 12, 12),
        Paint()
          ..color = Color.fromARGB(
            255,
            (i * 37) % 256,
            (j * 53) % 256,
            ((i + j) * 71) % 256,
          ),
      );
    }
  }
  return recorder.endRecording().toImage(_width, _height);
}

void main() {
  testWidgets('the gradient draws what it drew when the shader hashed its '
      'noise for every pixel', (tester) async {
    await tester.runAsync(() async {
      final program = await ui.FragmentProgram.fromAsset(
        'lib/shaders/gradient.frag',
      );
      final covers = await _covers();
      final lattice = NoiseLattice();

      for (var d = 0; d < _drawings.length; d++) {
        final (time, mouse, isDark) = _drawings[d];
        final shader = program.fragmentShader();
        setGradientUniforms(
          shader,
          image: covers,
          size: const Size(_width + 0.0, _height + 0.0),
          time: time,
          mouse: mouse,
          gradientParams: gradientParams,
          effectParams: effectParams,
          color: _color,
          color2: _color2,
          isDark: isDark,
          lattice: lattice,
        );
        final recorder = ui.PictureRecorder();
        Canvas(recorder).drawRect(
          const Rect.fromLTWH(0, 0, _width + 0.0, _height + 0.0),
          Paint()..shader = shader,
        );
        final image = await recorder.endRecording().toImage(_width, _height);
        final bytes = (await image.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        ))!.buffer.asUint8List();

        for (var j = 0; j < 5; j++) {
          for (var i = 0; i < 6; i++) {
            final x = 7 + i * 16, y = 6 + j * 15;
            final at = (y * _width + x) * 4;
            final expected = _drawn[d][j * 6 + i];
            for (var c = 0; c < 4; c++) {
              final want = (expected >> (24 - 8 * c)) & 0xff;
              expect(
                (bytes[at + c] - want).abs(),
                lessThanOrEqualTo(1),
                reason: 'drawing $d at ($x, $y), channel $c',
              );
            }
          }
        }
      }
    });
  });
}
