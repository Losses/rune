import 'dart:math';
import 'dart:typed_data';
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

/// The gradient on a grid of [fieldSize] pixels, or the app's without one.
Future<CoverWallGradient> _gradient({int? fieldSize}) async {
  final field = await ui.FragmentProgram.fromAsset(
    'lib/shaders/gradient_field.frag',
  );
  final composite = await ui.FragmentProgram.fromAsset(
    'lib/shaders/gradient.frag',
  );
  return fieldSize == null
      ? CoverWallGradient(field.fragmentShader(), composite.fragmentShader())
      : CoverWallGradient(
          field.fragmentShader(),
          composite.fragmentShader(),
          fieldSize: fieldSize,
        );
}

Future<Uint8List> _draw(
  CoverWallGradient gradient,
  ui.Image covers,
  Size size, {
  required double time,
  required Offset mouse,
  required bool isDark,
  required NoiseLattice lattice,
}) async {
  final recorder = ui.PictureRecorder();
  gradient.paint(
    Canvas(recorder),
    size,
    covers,
    time: time,
    mouse: mouse,
    gradientParams: gradientParams,
    effectParams: effectParams,
    color: _color,
    color2: _color2,
    isDark: isDark,
    lattice: lattice,
  );
  final image = await recorder.endRecording().toImage(
    size.width.toInt(),
    size.height.toInt(),
  );
  return (await image.toByteData(
    format: ui.ImageByteFormat.rawRgba,
  ))!.buffer.asUint8List();
}

/// Squares of every brightness, white among them, at the size of a screen.
Future<ui.Image> _screenOfCovers() {
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  for (var i = 0; i < 10; i++) {
    for (var j = 0; j < 8; j++) {
      final level = ((i * 8 + j) * 37) % 256;
      canvas.drawRect(
        Rect.fromLTWH(i * 64.0, j * 60.0, 64, 60),
        Paint()..color = Color.fromARGB(255, level, 255 - level, level ~/ 2),
      );
    }
  }
  canvas.drawRect(
    const Rect.fromLTWH(256, 180, 128, 120),
    Paint()..color = const Color(0xFFFFFFFF),
  );
  return recorder.endRecording().toImage(640, 480);
}

void main() {
  testWidgets('cover gradients composite transparent and bright samples', (
    tester,
  ) async {
    await tester.runAsync(() async {
      const bounds = Rect.fromLTWH(0, 0, 3, 1);
      const backdrop = Color(0xFF102030);
      final source = ui.PictureRecorder();
      Canvas(source)
        ..drawRect(
          const Rect.fromLTWH(1, 0, 1, 1),
          Paint()..color = const Color(0xFFFFFFFF),
        )
        ..drawRect(
          const Rect.fromLTWH(2, 0, 1, 1),
          Paint()..color = const Color(0x80000000),
        );
      final coverPicture = source.endRecording();
      final covers = await coverPicture.toImage(3, 1);
      coverPicture.dispose();

      final swatch = ui.PictureRecorder();
      Canvas(swatch).drawColor(const Color(0xFF4080C0), BlendMode.src);
      final fieldPicture = swatch.endRecording();
      final field = await fieldPicture.toImage(1, 1);
      fieldPicture.dispose();

      final program = await ui.FragmentProgram.fromAsset(
        'lib/shaders/gradient.frag',
      );
      final shader = program.fragmentShader()
        ..setFloat(0, bounds.width)
        ..setFloat(1, bounds.height)
        ..setFloat(4, 0.5)
        ..setFloat(5, 0.5)
        ..setImageSampler(0, covers)
        ..setImageSampler(1, field);
      for (final (isDark, expected) in const [
        (false, [0x102030FF, 0x102030FF, 0x285078FF]),
        (true, [0x102030FF, 0x4080C0FF, 0x081018FF]),
      ]) {
        shader.setFloat(6, isDark ? 1 : 0);
        final recorder = ui.PictureRecorder();
        Canvas(recorder)
          ..drawColor(backdrop, BlendMode.src)
          ..drawRect(bounds, Paint()..shader = shader);
        final picture = recorder.endRecording();
        final rendered = await picture.toImage(3, 1);
        final bytes = (await rendered.toByteData())!;
        rendered.dispose();
        picture.dispose();
        for (var i = 0; i < expected.length; i++) {
          const samples = [
            'gap',
            'white cover',
            'half-transparent black cover',
          ];
          expect(
            bytes.getUint32(i * 4),
            expected[i],
            reason: '${isDark ? 'dark' : 'light'} theme: ${samples[i]}',
          );
        }
      }
      shader.dispose();
      covers.dispose();
      field.dispose();
    });
  });

  testWidgets('the gradient draws what it drew when the shader hashed its '
      'noise for every pixel, and before its colour field was worked out on '
      'a grid of its own', (tester) async {
    await tester.runAsync(() async {
      // The drawing is smaller than the field's grid, so the field is worked
      // out for every pixel, as the single shader did.
      final gradient = await _gradient(fieldSize: _width);
      final covers = await _covers();
      final lattice = NoiseLattice();

      for (var d = 0; d < _drawings.length; d++) {
        final (time, mouse, isDark) = _drawings[d];
        final bytes = await _draw(
          gradient,
          covers,
          const Size(_width + 0.0, _height + 0.0),
          time: time,
          mouse: mouse,
          isDark: isDark,
          lattice: lattice,
        );

        for (var j = 0; j < 5; j++) {
          for (var i = 0; i < 6; i++) {
            final x = 7 + i * 16, y = 6 + j * 15;
            final at = (y * _width + x) * 4;
            final expected = _drawn[d][j * 6 + i];
            final alpha = expected & 0xff;
            for (var c = 0; c < 4; c++) {
              final baseline = (expected >> (24 - 8 * c)) & 0xff;
              // The historical light-theme RGB values are straight colour.
              final want = !isDark && c < 3
                  ? (baseline * alpha / 255).round()
                  : baseline;
              expect(
                (bytes[at + c] - want).abs(),
                lessThanOrEqualTo(1),
                reason: 'drawing $d at ($x, $y), channel $c',
              );
            }
          }
        }
      }
      gradient.dispose();
    });
  });

  testWidgets('working the colour field out on its grid of 320 pixels changes '
      'no pixel of a screen of covers by more than a level, whatever the '
      'screen\'s shape', (tester) async {
    await tester.runAsync(() async {
      final app = await _gradient();
      final covers = await _screenOfCovers();
      final lattice = NoiseLattice();
      final results = <String>[];

      // The field is laid out in the wall's own coordinates, so the grid along
      // the shorter side is what counts: 241 cells at 4:3, 138 at 21:9 and 149
      // on a phone held upright. 650 by 490 is not a whole number of grid
      // cells, so the field image reaches past the drawing.
      for (final size in const [
        Size(650, 490),
        Size(1680, 720),
        Size(390, 844),
      ]) {
        final exact = await _gradient(
          fieldSize: max(size.width, size.height).toInt(),
        );
        for (final (time, mouse, isDark) in [
          (0.37, Offset(size.width / 2, size.height / 2), true),
          (37.25, Offset(size.width * 0.15, size.height * 0.8), true),
          (123.45, Offset(size.width * 0.9, size.height * 0.1), true),
          (286.5, const Offset(-1000, -1000), true),
          (37.25, Offset(size.width * 0.15, size.height * 0.8), false),
          (123.45, Offset(size.width * 0.9, size.height * 0.1), false),
        ]) {
          final want = await _draw(
            exact,
            covers,
            size,
            time: time,
            mouse: mouse,
            isDark: isDark,
            lattice: lattice,
          );
          final got = await _draw(
            app,
            covers,
            size,
            time: time,
            mouse: mouse,
            isDark: isDark,
            lattice: lattice,
          );
          var largest = 0, overOne = 0;
          for (var p = 0; p < want.length; p += 4) {
            var pixel = 0;
            for (var c = 0; c < 4; c++) {
              pixel = max(pixel, (want[p + c] - got[p + c]).abs());
            }
            largest = max(largest, pixel);
            if (pixel > 1) overOne++;
          }
          results.add(
            '$size t $time dark $isDark: largest $largest, over 1: $overOne',
          );
          expect(largest, lessThanOrEqualTo(1), reason: results.last);
        }
        exact.dispose();
      }
      // Moving the field image half a grid cell, scaling it up without
      // filtering, or a grid of 160 pixels breaks this bound.
      app.dispose();
    });
  });
}
