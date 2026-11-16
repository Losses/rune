import 'dart:math';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/playback_controller/fft_visualize.dart';

const _width = 640.0, _height = 160.0;
// Room around the bars' box, so the glow that spills out of it is compared too.
const _margin = 48.0;
const _color = Color(0xFF0078D4);

/// What the spectrum drew before its glow became a shader: solid bars, as
/// lines as wide as a band, under a blur of 12 and an opacity of 0.87.
class _Bars extends CustomPainter {
  _Bars(this.values);

  final List<double> values;

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = _color
      ..strokeWidth = size.width / values.length;
    final midY = size.height / 2;
    for (int i = 0; i < values.length; i++) {
      final x = i * (size.width / values.length);
      final y = values[i] * size.height / 2;
      canvas.drawLine(Offset(x, midY - y), Offset(x, midY + y), paint);
    }
  }

  @override
  bool shouldRepaint(_Bars oldDelegate) => true;
}

Widget _framed(Key key, Widget bars) => Directionality(
  textDirection: TextDirection.ltr,
  child: Align(
    alignment: Alignment.topLeft,
    child: RepaintBoundary(
      key: key,
      child: SizedBox(
        width: _width + 2 * _margin,
        height: _height + 2 * _margin,
        child: Padding(padding: const EdgeInsets.all(_margin), child: bars),
      ),
    ),
  ),
);

Future<Uint8List> _pixels(WidgetTester tester, Key key) async {
  final boundary =
      tester.renderObject(find.byKey(key)) as RenderRepaintBoundary;
  return (await tester.runAsync(() async {
    final image = await boundary.toImage();
    final data = await image.toByteData();
    image.dispose();
    return data!.buffer.asUint8List();
  }))!;
}

typedef _Difference = ({int largest, double overTwo, int lost});

/// How [after] differs from [before]: the largest difference of any channel,
/// the share of pixels that differ by more than 2, and how many pixels where
/// [before] shows the glow (alpha 2 or more) [after] leaves empty.
_Difference _compare(Uint8List before, Uint8List after) {
  var largest = 0, overTwo = 0, lost = 0;
  for (int p = 0; p < before.length; p += 4) {
    var pixel = 0;
    for (int c = 0; c < 4; c++) {
      pixel = max(pixel, (before[p + c] - after[p + c]).abs());
    }
    largest = max(largest, pixel);
    if (pixel > 2) overTwo++;
    if (before[p + 3] >= 2 && after[p + 3] == 0) lost++;
  }
  return (largest: largest, overTwo: overTwo / (before.length / 4), lost: lost);
}

/// 64 bands the way music fills them: tall at the bottom, falling off, rough.
List<double> _spectrum(int seed) {
  final random = Random(seed);
  return List.generate(
    64,
    (i) => (0.9 * exp(-i / 18) + 0.25 * random.nextDouble()).clamp(0.0, 1.0),
  );
}

void main() {
  final cases = <String, List<double>>{
    'music': _spectrum(1),
    'other music': _spectrum(7),
    'bars reaching the box edges': List.generate(64, (i) => i.isEven ? 1.0 : 0),
    'a lone bar': List.generate(64, (i) => i == 20 ? 0.6 : 0),
    'quiet': List.generate(64, (i) => 0.04 * (i % 5)),
  };

  testWidgets('the shader draws the glow the blurred, faded bars drew, and '
      'drawing it at a third of the resolution changes it by 2 levels at most', (
    tester,
  ) async {
    final program = (await tester.runAsync(
      () => ui.FragmentProgram.fromAsset('lib/shaders/spectrum.frag'),
    ))!;
    final glow = SpectrumGlow(program.fragmentShader());
    final fullGlow = SpectrumGlow(program.fragmentShader(), resolution: 1);
    addTearDown(glow.dispose);
    addTearDown(fullGlow.dispose);
    final results = <String, _Difference>{};
    final scaling = <String, _Difference>{};

    for (final MapEntry(key: name, value: values) in cases.entries) {
      const before = Key('before'), after = Key('after'), full = Key('full');
      final bars = FFTBars()..values = values;
      addTearDown(bars.dispose);

      await tester.pumpWidget(
        _framed(
          before,
          Opacity(
            opacity: SpectrumGlow.glowOpacity,
            child: ImageFiltered(
              imageFilter: ui.ImageFilter.blur(
                sigmaX: SpectrumGlow.blurSigma,
                sigmaY: SpectrumGlow.blurSigma,
              ),
              child: CustomPaint(painter: _Bars(values)),
            ),
          ),
        ),
      );
      final drawn = await _pixels(tester, before);

      await tester.pumpWidget(
        _framed(after, CustomPaint(painter: FFTPainter(bars, _color, glow))),
      );
      final drawing = await _pixels(tester, after);

      await tester.pumpWidget(
        _framed(full, CustomPaint(painter: FFTPainter(bars, _color, fullGlow))),
      );
      final fullDrawing = await _pixels(tester, full);

      results[name] = _compare(drawn, fullDrawing);
      scaling[name] = _compare(fullDrawing, drawing);
    }

    // The blur filter is itself an approximation of a Gaussian (in tests,
    // Skia's software blur), which the shader computes to 3 deviations: they
    // differ by up to 5 levels, along the edges of the glow, in about 2% of
    // the pixels. Drawing the bars half a pixel off, a deviation of 11, an
    // opacity of 1, the middle line a pixel off, the glow only to 2
    // deviations, or the shader's sum only to 2 deviations breaks one of
    // these bounds.
    expect(
      results.values.every(
        (r) => r.largest <= 5 && r.overTwo <= 0.025 && r.lost == 0,
      ),
      isTrue,
      reason: '$results',
    );
    // Working the glow out at a third of the resolution, as the app does, and
    // scaling it back up moves no pixel by more than 2 levels, which the image's rounding to
    // whole levels before and after the scaling accounts for (few pixels move
    // by 2: under 0.02% over 16 spectra, the bars a fraction of a pixel off or
    // not). Drawing the small image a pixel and a half off, or scaling it up
    // without filtering, breaks this bound.
    expect(
      scaling.values.every((r) => r.largest <= 2 && r.lost == 0),
      isTrue,
      reason: '$scaling',
    );
  });

  testWidgets('the glow is worked out in an image of the bars\' area at its '
      'resolution, a third of a pixel for each logical pixel unless told', (
    tester,
  ) async {
    final program = (await tester.runAsync(
      () => ui.FragmentProgram.fromAsset('lib/shaders/spectrum.frag'),
    ))!;
    final bars = FFTBars()..values = cases['a lone bar']!;
    addTearDown(bars.dispose);

    // 64 bars over 640 pixels are 10 wide, and the bar of 0.6 reaches 48 up
    // and down from the middle at 80; the area is that grown by 3 deviations,
    // 36: from -41 to 671 across and from -4 to 164 down, 712 by 168. At a
    // third it is grown to whole pixels of the image, from -42 to 672 and
    // from -6 to 165: 238 by 57.
    final sizes = <String, Size?>{};
    for (final (name, glow) in [
      ('2', SpectrumGlow(program.fragmentShader(), resolution: 2)),
      ('1', SpectrumGlow(program.fragmentShader(), resolution: 1)),
      ('default', SpectrumGlow(program.fragmentShader())),
    ]) {
      addTearDown(glow.dispose);
      await tester.pumpWidget(
        _framed(
          Key(name),
          CustomPaint(painter: FFTPainter(bars, _color, glow)),
        ),
      );
      sizes[name] = glow.lastImageSize;
    }
    expect(sizes, {
      '2': const Size(1424, 336),
      '1': const Size(712, 168),
      'default': const Size(238, 57),
    });
  });
}
