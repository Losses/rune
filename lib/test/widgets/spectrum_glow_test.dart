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

/// What the spectrum drew before its glow became a shader: solid bars as wide
/// as a band, under a blur of 12 and an opacity of 0.87.
///
/// The bars are one path. The app drew each as a line of its own, and where
/// two narrow bars share a pixel, blending their partial coverage one after
/// the other loses some of it: 512 bars over 640 pixels lost 12% of the glow,
/// in seams up to 31 levels darker. One path covers each pixel by the bars'
/// true area, which the shaders work out.
class _Bars extends CustomPainter {
  _Bars(this.values);

  final List<double> values;

  @override
  void paint(Canvas canvas, Size size) {
    final barWidth = size.width / values.length;
    final midY = size.height / 2;
    final bars = Path();
    for (int i = 0; i < values.length; i++) {
      final y = values[i] * size.height / 2;
      bars.addRect(
        Rect.fromLTRB(
          (i - 0.5) * barWidth,
          midY - y,
          (i + 0.5) * barWidth,
          midY + y,
        ),
      );
    }
    canvas.drawPath(bars, Paint()..color = _color);
  }

  @override
  bool shouldRepaint(_Bars oldDelegate) => true;
}

/// The bars as the spectrum drew them before its glow became shaders
/// (d50ea3ef), each a line as wide as a band, drawn one after another.
class _Lines extends CustomPainter {
  _Lines(this.values);

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
  bool shouldRepaint(_Lines oldDelegate) => true;
}

/// The glow before the shaders: [_Lines] under a blur of 12 and an opacity
/// of 0.87.
Widget _oldGlow(List<double> values) => Opacity(
  opacity: 0.87,
  child: ImageFiltered(
    imageFilter: ui.ImageFilter.blur(sigmaX: 12, sigmaY: 12),
    child: CustomPaint(painter: _Lines(values)),
  ),
);

Widget _framed(Key key, Widget bars, {double height = _height}) =>
    Directionality(
      textDirection: TextDirection.ltr,
      child: Align(
        alignment: Alignment.topLeft,
        child: RepaintBoundary(
          key: key,
          child: SizedBox(
            width: _width + 2 * _margin,
            height: height + 2 * _margin,
            child: Padding(padding: const EdgeInsets.all(_margin), child: bars),
          ),
        ),
      ),
    );

/// The old glow: [_Bars] under the blur filter and the opacity layer.
Widget _blurredBars(List<double> values) => Opacity(
  opacity: SpectrumGlow.glowOpacity,
  child: ImageFiltered(
    imageFilter: ui.ImageFilter.blur(
      sigmaX: SpectrumGlow.blurSigma,
      sigmaY: SpectrumGlow.blurSigma,
    ),
    child: CustomPaint(painter: _Bars(values)),
  ),
);

typedef _Glows = SpectrumGlow Function({double resolution});

/// Loads the glow's two shaders, and gives glows that draw with them.
Future<_Glows> _glows(WidgetTester tester) async {
  final programs = (await tester.runAsync(
    () => Future.wait([
      ui.FragmentProgram.fromAsset('lib/shaders/spectrum_bars.frag'),
      ui.FragmentProgram.fromAsset('lib/shaders/spectrum_glow.frag'),
    ]),
  ))!;
  return ({double resolution = 4 / SpectrumGlow.blurSigma}) => SpectrumGlow(
    programs[0].fragmentShader(),
    programs[1].fragmentShader(),
    resolution: resolution,
  );
}

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

/// What the player sends while music plays (playback/src/realtime_fft.rs,
/// with a window of 512 in playback/src/internal.rs): the magnitudes of a
/// 512 point FFT, mirrored about the middle band, weighted by a Nuttall window
/// and scaled to a peak of 1, so that the outer bands stay near 0.
List<double> _playerSpectrum(int seed) {
  final random = Random(seed);
  final magnitudes = List.generate(
    257,
    (k) => exp(-k / 40) * (0.5 + random.nextDouble()),
  );
  final weighted = List.generate(512, (n) {
    final f = 2 * pi * n / 511;
    final window =
        0.355768 -
        0.487396 * cos(f) +
        0.144232 * cos(2 * f) -
        0.012604 * cos(3 * f);
    return magnitudes[n <= 256 ? n : 512 - n] * window;
  });
  final peak = weighted.reduce(max);
  return [for (final value in weighted) value / peak];
}

/// The share of a Gaussian of deviation 1 that lies below [z], with erf by
/// Abramowitz and Stegun 7.1.26 (error under 1.5e-7).
double _below(double z) {
  final x = z / sqrt2;
  final a = x.abs();
  final t = 1 / (1 + 0.3275911 * a);
  final erf =
      1 -
      (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t -
                      0.284496736) *
                  t +
              0.254829592) *
          t *
          exp(-a * a);
  return 0.5 + 0.5 * (x < 0 ? -erf : erf);
}

/// The glow's alpha in levels of 255 at ([x], [y]) in the bars' box, worked
/// out from what it is: the bars under a Gaussian blur of
/// [SpectrumGlow.blurSigma], faded to [SpectrumGlow.glowOpacity]. Bars more
/// than 5 deviations away across add under 3e-7 each and are left out.
double _exactAlpha(List<double> values, double x, double y) {
  const sigma = SpectrumGlow.blurSigma;
  const middle = _height / 2;
  final barWidth = _width / values.length;
  var glow = 0.0;
  for (var i = 0; i < values.length; i++) {
    final left = (i - 0.5) * barWidth;
    if (left + barWidth < x - 5 * sigma || left > x + 5 * sigma) continue;
    final h = values[i] * _height / 2;
    final across =
        _below((left + barWidth - x) / sigma) - _below((left - x) / sigma);
    final along =
        _below((middle + h - y) / sigma) - _below((middle - h - y) / sigma);
    glow += across * along;
  }
  return 255 * SpectrumGlow.glowOpacity * glow;
}

void main() {
  final cases = <String, List<double>>{
    'music': _spectrum(1),
    'other music': _spectrum(7),
    'the player\'s 512 bands': _playerSpectrum(1),
    'other 512 bands': _playerSpectrum(7),
    'bars reaching the box edges': List.generate(64, (i) => i.isEven ? 1.0 : 0),
    'a lone bar': List.generate(64, (i) => i == 20 ? 0.6 : 0),
    'quiet': List.generate(64, (i) => 0.04 * (i % 5)),
  };

  testWidgets('on a screen 640 pixels wide at a pixel ratio of 1, the glow is '
      'as bright as the glow before the shaders', (tester) async {
    final glows = await _glows(tester);
    final glow = glows();
    addTearDown(glow.dispose);
    final results = <String, ({double brightness, _Difference difference})>{};

    for (final seed in [1, 7, 11]) {
      final values = _playerSpectrum(seed);
      final bars = FFTBars()..values = values;
      addTearDown(bars.dispose);
      final before = Key('before $seed'), after = Key('after $seed');
      await tester.pumpWidget(_framed(before, _oldGlow(values)));
      final drawn = await _pixels(tester, before);
      await tester.pumpWidget(
        _framed(after, CustomPaint(painter: FFTPainter(bars, _color, glow))),
      );
      final drawing = await _pixels(tester, after);
      var drawnAlpha = 0, drawingAlpha = 0;
      for (var p = 3; p < drawn.length; p += 4) {
        drawnAlpha += drawn[p];
        drawingAlpha += drawing[p];
      }
      results['seed $seed'] = (
        brightness: drawingAlpha / drawnAlpha,
        difference: _compare(drawn, drawing),
      );
    }

    // The old glow's seams took 11% off where the bars are dense and nothing
    // where they stand apart; the opacity takes the same off everywhere, so
    // the two differ by up to 7 levels in about 13% of the pixels, and in
    // all by under half a percent. The glow at the old opacity of 0.87 is
    // 12% brighter, which breaks the bound.
    expect(
      results.values.every(
        (r) =>
            (r.brightness - 1).abs() <= 0.01 &&
            r.difference.largest <= 8 &&
            r.difference.lost == 0,
      ),
      isTrue,
      reason: '$results',
    );
  });

  testWidgets('the shaders draw the glow the blurred, faded bars drew, and '
      'drawing it at a third of the resolution changes it by 2 levels at most', (
    tester,
  ) async {
    final glows = await _glows(tester);
    final glow = glows();
    final fullGlow = glows(resolution: 1);
    addTearDown(glow.dispose);
    addTearDown(fullGlow.dispose);
    final results = <String, _Difference>{};
    final scaling = <String, _Difference>{};

    for (final MapEntry(key: name, value: values) in cases.entries) {
      const before = Key('before'), after = Key('after'), full = Key('full');
      final bars = FFTBars()..values = values;
      addTearDown(bars.dispose);

      await tester.pumpWidget(_framed(before, _blurredBars(values)));
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
    // Skia's software blur), which the shaders compute to 3 deviations: they
    // differ by up to 5 levels, along the edges of the glow, in about 2% of
    // the pixels. Shaders given a deviation of 11, the glow at an opacity of
    // 1, the middle line a pixel off, or the bars blurred up and down or
    // across only to 2 deviations breaks one of these bounds.
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

  testWidgets(
    'the two passes draw the bars\' Gaussian blur within 1.5 '
    'levels of it worked out exactly, at the full resolution and at a third',
    (tester) async {
      final glows = await _glows(tester);
      final largest = <String, double>{};

      for (final MapEntry(key: name, value: values) in cases.entries) {
        for (final (label, glow) in [
          ('full', glows(resolution: 1)),
          ('third', glows()),
        ]) {
          addTearDown(glow.dispose);
          final bars = FFTBars()..values = values;
          addTearDown(bars.dispose);
          final key = Key('$name $label');
          await tester.pumpWidget(
            _framed(key, CustomPaint(painter: FFTPainter(bars, _color, glow))),
          );
          final pixels = await _pixels(tester, key);
          final columns = (_width + 2 * _margin).toInt();
          var difference = 0.0;
          // Every third pixel across and down.
          for (var row = 0; row < _height + 2 * _margin; row += 3) {
            for (var column = 0; column < columns; column += 3) {
              final alpha = pixels[(row * columns + column) * 4 + 3];
              final exact = _exactAlpha(
                values,
                column + 0.5 - _margin,
                row + 0.5 - _margin,
              );
              difference = max(difference, (alpha - exact).abs());
            }
          }
          largest['$name, $label'] = difference;
        }
      }

      // Rounding the glow to whole levels moves it by up to half a level, and
      // the image of the first pass by up to another half (0.92 at the most
      // here); scaling the image up from a third moves a pixel by a little more
      // (1.11 at the most). Sharing each bar with its own column alone (a box
      // for the tent), a tent half as wide, weights 2% heavy, a blur across of
      // 2 deviations, or the columns read half a pixel off breaks these bounds.
      expect(
        largest.entries.every(
          (r) => r.value <= (r.key.endsWith('full') ? 1.25 : 1.5),
        ),
        isTrue,
        reason: '$largest',
      );
    },
  );

  testWidgets('the halves above and below the middle line meet without a '
      'seam where the line crosses a row of pixels', (tester) async {
    final glows = await _glows(tester);
    final glow = glows();
    addTearDown(glow.dispose);
    final values = _playerSpectrum(3);
    final bars = FFTBars()..values = values;
    addTearDown(bars.dispose);
    const before = Key('before'), after = Key('after');

    // A box 161 high puts the middle line through the middle of a row.
    await tester.pumpWidget(_framed(before, _blurredBars(values), height: 161));
    final drawn = await _pixels(tester, before);
    await tester.pumpWidget(
      _framed(
        after,
        CustomPaint(painter: FFTPainter(bars, _color, glow)),
        height: 161,
      ),
    );
    final drawing = await _pixels(tester, after);

    // The bounds of the comparison above. Drawing the halves antialiased
    // darkens the row the line crosses, by 49 levels here, and flipping the
    // lower half about a line a pixel off leaves a row empty: either breaks
    // these bounds.
    final difference = _compare(drawn, drawing);
    expect(
      difference.largest <= 5 &&
          difference.overTwo <= 0.025 &&
          difference.lost == 0,
      isTrue,
      reason: '$difference',
    );
  });

  testWidgets('the glow is worked out in an image of the upper half of the '
      'bars\' area at its resolution, a third of a pixel for each logical '
      'pixel unless told', (tester) async {
    final glows = await _glows(tester);
    final bars = FFTBars()..values = cases['a lone bar']!;
    addTearDown(bars.dispose);

    // 64 bars over 640 pixels are 10 wide, and the bar of 0.6 reaches 48 up
    // from the middle at 80; the upper half of the area is that grown by 3
    // deviations, 36: from -41 to 671 across and from -4 down to the middle,
    // 712 by 84. At a third it is grown to whole pixels of the image, from
    // -42 to 672 across and 84 up from the middle: 238 by 28.
    final sizes = <String, Size?>{};
    for (final (name, glow) in [
      ('2', glows(resolution: 2)),
      ('1', glows(resolution: 1)),
      ('default', glows()),
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
      '2': const Size(1424, 168),
      '1': const Size(712, 84),
      'default': const Size(238, 28),
    });
  });
}
