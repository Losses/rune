import 'dart:math';
import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_manager.dart';
import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

class FFTVisualizerRegistry {
  static final FFTVisualizerRegistry _instance =
      FFTVisualizerRegistry._internal();
  factory FFTVisualizerRegistry() => _instance;

  late final _windowObserver = _WindowVisibilityObserver(this);

  FFTVisualizerRegistry._internal() {
    WidgetsBinding.instance.addObserver(_windowObserver);
  }

  final Set<FFTVisualizeState> _visualizers = {};
  bool _isWindowVisible = true;

  void register(FFTVisualizeState visualizer) {
    _visualizers.add(visualizer);
    _updateFFTCalculation();
  }

  void unregister(FFTVisualizeState visualizer) {
    _visualizers.remove(visualizer);
    _updateFFTCalculation();
  }

  void setWindowVisibility(bool isVisible) {
    _isWindowVisible = isVisible;
    _updateFFTCalculation();
  }

  void _updateFFTCalculation() {
    final shouldCalculateFFT = _visualizers.isNotEmpty && _isWindowVisible;

    SetRealtimeFFTEnabledRequest(
      enabled: shouldCalculateFFT,
    ).sendSignalToRust();
  }

  void dispose() {
    WidgetsBinding.instance.removeObserver(_windowObserver);
  }
}

class FFTVisualize extends StatefulWidget {
  const FFTVisualize({super.key, this.alignment = Alignment.topCenter});

  /// Where the bars, drawn twice as tall as the parent, sit against it. At the
  /// top their middle line is the parent's bottom edge, so that only their
  /// upper half rises over it; at the bottom the middle line is the parent's
  /// top edge, and only the lower half hangs under it.
  final Alignment alignment;

  @override
  FFTVisualizeState createState() => FFTVisualizeState();
}

double tanh(double angle) {
  if (angle > 19.1) {
    return 1.0;
  }

  if (angle < -19.1) {
    return -1.0;
  }

  var e1 = exp(angle);
  var e2 = exp(-angle);
  return (e1 - e2) / (e1 + e2);
}

/// The bars the spectrum shows, changed every frame while music plays.
///
/// They reach [FFTPainter] through this notifier, which repaints the bars
/// alone. A rebuild for each change would rebuild inside the layout builders
/// above the spectrum, and such a rebuild lays out every box up to the nearest
/// relayout boundary; layout marks each of them for paint, so the page around
/// the spectrum would repaint every frame too.
class FFTBars extends ChangeNotifier {
  List<double> values = [];

  void changed() => notifyListeners();
}

class FFTVisualizeState extends State<FFTVisualize>
    with SingleTickerProviderStateMixin {
  final _bars = FFTBars();
  SpectrumGlow? _glow;
  List<double> _targetFftValues = [];
  late final Ticker _ticker = createTicker(_onTick);
  bool _hasData = false;
  int _lastUpdateTime = 0;
  final _registry = FFTVisualizerRegistry();
  bool _mildSpectrum = false;
  StreamSubscription? _mildSpectrumSubscription;
  StreamSubscription? _fftSubscription;

  @override
  void initState() {
    super.initState();

    _registry.register(this);
    _loadGlow();

    SettingsManager().getValue<String>(kMildSpectrumKey).then((x) {
      if (x == 'true') {
        _mildSpectrum = true;
      }
    });

    _mildSpectrumSubscription = SettingsManager().listenValue<String>(
      kMildSpectrumKey,
      (x) {
        _mildSpectrum = x == 'true' ? true : false;
      },
    );

    _fftSubscription = RealtimeFFT.rustSignalStream.listen((rustSignal) {
      if (!mounted) return;

      _targetFftValues = _mildSpectrum
          ? rustSignal.message.value.map(tanh).toList()
          : rustSignal.message.value;
      _lastUpdateTime = DateTime.now().millisecondsSinceEpoch;
      if (!_hasData) {
        _hasData = true;
        if (!_ticker.isTicking) {
          _ticker.start();
        }
      }
    });
  }

  void _onTick(Duration elapsed) {
    final now = DateTime.now().millisecondsSinceEpoch;
    if (now - _lastUpdateTime > 168) {
      final current = _bars.values;
      final reduced = current.fold<double>(0.0, (a, b) => a + b);
      if (!_hasData && reduced < 1e-2) {
        // The bars have fallen: draw them flat once and stop until the next
        // spectrum arrives.
        _bars.values = List.filled(current.length, 0.0);
        _bars.changed();
        _ticker.stop();
        return;
      }

      _targetFftValues = List.filled(current.length, 0.0);
      _hasData = false;
    }

    _lerpedFftValues();
    _bars.changed();
  }

  Future<void> _loadGlow() async {
    final [bars, glow] = await Future.wait([
      ui.FragmentProgram.fromAsset('lib/shaders/spectrum_bars.frag'),
      ui.FragmentProgram.fromAsset('lib/shaders/spectrum_glow.frag'),
    ]);
    if (!mounted) return;
    setState(
      () => _glow = SpectrumGlow(bars.fragmentShader(), glow.fragmentShader()),
    );
  }

  @override
  dispose() {
    _glow?.dispose();
    _fftSubscription?.cancel();
    _mildSpectrumSubscription?.cancel();
    _ticker.dispose();
    _bars.dispose();
    _registry.unregister(this);
    _registry.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final color = FluentTheme.of(context).accentColor;

    return LayoutBuilder(
      builder: (context, constraints) {
        double parentHeight = constraints.maxHeight;

        return OverflowBox(
          maxHeight: parentHeight * 2,
          alignment: widget.alignment,
          child: SizedBox(
            height: parentHeight * 2,
            child: CustomPaint(painter: FFTPainter(_bars, color, _glow)),
          ),
        );
      },
    );
  }

  void _lerpedFftValues() {
    if (_targetFftValues.isEmpty) return;

    var current = _bars.values;
    if (current.length != _targetFftValues.length) {
      current = _bars.values = List.filled(_targetFftValues.length, 0.0);
    }

    for (int i = 0; i < current.length; i++) {
      current[i] = ui.lerpDouble(
        current[i],
        _targetFftValues[i],
        _mildSpectrum ? 0.08 : 0.2,
      )!;
    }
  }
}

class _WindowVisibilityObserver extends WidgetsBindingObserver {
  final FFTVisualizerRegistry _registry;

  _WindowVisibilityObserver(this._registry);

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _registry.setWindowVisibility(state == AppLifecycleState.resumed);
  }
}

/// The spectrum's glow: the bars, Gaussian blurred by [blurSigma] and faded
/// to [glowOpacity], which two shaders work out in two passes.
///
/// The glow used to be the bars drawn into a layer that a blur filter and an
/// opacity layer went over every frame. The shaders give the same pixels,
/// drawn only over the bars' bounds grown by three deviations, the area the
/// blur filter drew into: a frame repaints no more of the screen than the glow
/// reaches, so a quiet spectrum repaints a low strip.
///
/// lib/shaders/spectrum_bars.frag blurs the bars up and down, exactly, at the
/// middle of each column of an image, sharing each bar between the columns
/// nearest it. lib/shaders/spectrum_glow.frag then blurs those columns across.
/// Worked out in one pass, each pixel summed every bar within three
/// deviations: 59 of the player's 512 bars on a screen 640 pixels wide. In
/// two, a pixel takes the few bars of its column, and then 25 columns. With
/// nothing else on the screen of a Radxa ZERO 3, its GPU at 400 MHz, the glow
/// in two passes loads the GPU 20%, in one 69%, and as the old blur filter
/// drew it 32%.
///
/// The image covers the area's upper half. It is drawn scaled up over that
/// half, and flipped over the lower half, which mirrors it. A blur of 12
/// leaves no detail finer than a few pixels, so the image has [resolution]
/// pixels for each logical pixel, 4 for each deviation: a third, on every
/// screen. The two passes stay within 1.5 levels of 255 of the blur worked
/// out exactly, at the full resolution and at a third
/// (lib/test/widgets/spectrum_glow_test.dart).
class SpectrumGlow {
  SpectrumGlow(
    this._barsShader,
    this._glowShader, {
    this.resolution = 4 / blurSigma,
  });

  static const double blurSigma = 12.0;

  /// The glow's opacity, which keeps it as bright as the glow before the
  /// shaders on the kiosk of a Radxa ZERO 3: a screen 640 pixels wide at a
  /// pixel ratio of 1.
  ///
  /// That glow drew the bars as lines, one after another, at an opacity of
  /// 0.87. Under Skia, two lines that share a pixel each cover part of it,
  /// and blending the second over the first leaves seams: how much they take
  /// off moves with a bar's width in pixels and with where the window sits
  /// on the pixel grid. The shaders draw the bars whole. On the kiosk, the
  /// one place where the old glow had a single look, it was 0.891 of theirs
  /// at 0.87 (on the board, the same spectrum drawn both ways), so the glow
  /// takes 0.87 * 0.891. On desktops under Skia the old glow lost 4 to 6% at
  /// a pixel ratio of 1 and up to 4% at 2, and under Impeller nothing, so
  /// there the glow is up to 11% fainter than it was.
  static const double glowOpacity = 0.775;

  /// The bars the shaders take: the player sends 512 bands, the magnitudes of
  /// a 512 point FFT (playback/src/internal.rs). lib/shaders/spectrum_bars.frag
  /// holds no more, so a longer spectrum fails an assertion here, as missing
  /// bands would leave the glow without them.
  static const int maxBars = 512;

  /// The columns on each side that lib/shaders/spectrum_glow.frag can take
  /// in.
  static const int maxReach = 72;

  /// lib/shaders/spectrum_bars.frag, the first pass.
  final ui.FragmentShader _barsShader;

  /// lib/shaders/spectrum_glow.frag, the second pass.
  final ui.FragmentShader _glowShader;

  /// The pixels of the glow's image for each logical pixel.
  final double resolution;

  /// The images the last frame drew, kept until the next one replaces them:
  /// the columns of the first pass and the glow of the second.
  ui.Image? _lastColumns;
  ui.Image? _lastImage;

  /// The size in pixels of the image the last frame drew.
  @visibleForTesting
  Size? get lastImageSize => _lastImage == null
      ? null
      : Size(_lastImage!.width.toDouble(), _lastImage!.height.toDouble());

  /// Draws the glow of [values] over [size].
  void paint(Canvas canvas, Size size, List<double> values, Color color) {
    if (values.isEmpty) return;
    assert(
      values.length <= maxBars,
      'The glow draws $maxBars bands at most, but the spectrum has '
      '${values.length}.',
    );

    final barWidth = size.width / values.length;
    final middle = size.height / 2;
    final count = min(values.length, maxBars);
    final alpha = color.a * glowOpacity;

    var tallest = 0.0;
    for (int i = 0; i < maxBars; i++) {
      final halfHeight = i < count ? (values[i] * size.height / 2).abs() : 0.0;
      _barsShader.setFloat(i, halfHeight);
      tallest = max(tallest, halfHeight);
    }
    if (tallest == 0) return;
    _barsShader
      ..setFloat(maxBars, barWidth)
      ..setFloat(maxBars + 1, middle)
      ..setFloat(maxBars + 2, count.toDouble())
      ..setFloat(maxBars + 3, blurSigma)
      ..setFloat(maxBars + 4, 1 / resolution);

    // The glow below the middle line is the glow above it upside down. The
    // image covers the upper half of the area the blur filter drew into, and
    // is drawn a second time flipped over the lower half, so the shaders work
    // out half the pixels. The half is grown to whole pixels of the image,
    // counted up from the middle line, where the two halves meet.
    final reach = 3 * blurSigma;
    final left = ((-barWidth / 2 - reach) * resolution).floor();
    final right = (((count - 0.5) * barWidth + reach) * resolution).ceil();
    final width = right - left;
    final height = ((tallest + reach) * resolution).ceil();
    final upper = Rect.fromLTRB(
      left / resolution,
      middle - height / resolution,
      right / resolution,
      middle,
    );
    final source = Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble());

    // The first pass reads the coordinates before the scaling, so each pixel
    // of its image is the bars at the middle of the block it stands for.
    final barsRecorder = ui.PictureRecorder();
    Canvas(barsRecorder)
      ..scale(resolution)
      ..translate(-upper.left, -upper.top)
      ..drawRect(upper, Paint()..shader = _barsShader);
    final barsPicture = barsRecorder.endRecording();
    final columns = barsPicture.toImageSync(width, height);
    barsPicture.dispose();

    // The first pass's tent spreads each bar as a Gaussian of 1 / sqrt(6)
    // columns does; the blur across leaves that out, so that the two spread
    // it by the glow's deviation.
    final columnSigma = sqrt(pow(blurSigma * resolution, 2) - 1 / 6);
    final columnReach = (3 * columnSigma).ceil();
    assert(
      columnReach <= maxReach,
      'The glow blurs $maxReach columns across at most, but a resolution of '
      '$resolution needs $columnReach.',
    );
    var weights = 0.0;
    for (int offset = -columnReach; offset <= columnReach; offset++) {
      weights += exp(-0.5 * pow(offset / columnSigma, 2));
    }
    _glowShader
      ..setFloat(0, color.r * alpha)
      ..setFloat(1, color.g * alpha)
      ..setFloat(2, color.b * alpha)
      ..setFloat(3, alpha)
      ..setFloat(4, width.toDouble())
      ..setFloat(5, height.toDouble())
      ..setFloat(6, columnSigma)
      ..setFloat(7, columnReach.toDouble())
      ..setFloat(8, 1 / weights)
      ..setImageSampler(0, columns);
    final glowRecorder = ui.PictureRecorder();
    Canvas(glowRecorder).drawRect(source, Paint()..shader = _glowShader);
    final glowPicture = glowRecorder.endRecording();
    final image = glowPicture.toImageSync(width, height);
    glowPicture.dispose();

    // Antialiased, the two halves would each cover part of a row of pixels
    // that the middle line crosses, and blending one over the other would
    // leave that row darker. Without, each pixel is in one half. The image's
    // other edges are 3 deviations from the bars, where the glow is under
    // half a level.
    final paint = Paint()
      ..filterQuality = FilterQuality.low
      ..isAntiAlias = false;
    canvas
      ..drawImageRect(image, source, upper, paint)
      ..save()
      ..translate(0, 2 * middle)
      ..scale(1, -1)
      ..drawImageRect(image, source, upper, paint)
      ..restore();
    _lastColumns?.dispose();
    _lastColumns = columns;
    _lastImage?.dispose();
    _lastImage = image;
  }

  void dispose() {
    _lastColumns?.dispose();
    _lastImage?.dispose();
    _barsShader.dispose();
    _glowShader.dispose();
  }
}

class FFTPainter extends CustomPainter {
  FFTPainter(this.bars, this.color, this.glow) : super(repaint: bars);

  final FFTBars bars;
  final Color color;

  /// The glow, or null while its shaders load.
  final SpectrumGlow? glow;

  @override
  void paint(Canvas canvas, Size size) =>
      glow?.paint(canvas, size, bars.values, color);

  @override
  bool shouldRepaint(FFTPainter oldDelegate) =>
      oldDelegate.bars != bars ||
      oldDelegate.color != color ||
      oldDelegate.glow != glow;
}
