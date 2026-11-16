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
    final program = await ui.FragmentProgram.fromAsset(
      'lib/shaders/spectrum.frag',
    );
    if (!mounted) return;
    setState(() => _glow = SpectrumGlow(program.fragmentShader()));
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
    final resolution = MediaQuery.devicePixelRatioOf(context);

    return LayoutBuilder(
      builder: (context, constraints) {
        double parentHeight = constraints.maxHeight;

        return OverflowBox(
          maxHeight: parentHeight * 2,
          alignment: widget.alignment,
          child: SizedBox(
            height: parentHeight * 2,
            child: CustomPaint(
              painter: FFTPainter(_bars, color, _glow, resolution: resolution),
            ),
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
/// to [glowOpacity], which lib/shaders/spectrum.frag works out for each pixel.
///
/// The glow used to be the bars drawn into a layer that a blur filter and an
/// opacity layer went over every frame. The shader gives the same pixels in
/// one pass, drawn only over the bars' bounds grown by three deviations, the
/// area the blur filter drew into: a frame repaints no more of the screen than
/// the glow reaches, so a quiet spectrum repaints a low strip.
///
/// The shader draws into an image of the area, which is then drawn over it
/// scaled: at the screen's own resolution that is the same picture, and an
/// image of fewer pixels spares the GPU. A blur of 12 leaves no detail finer
/// than a few pixels, so at a third of the screen's resolution the shader works
/// out a ninth of the pixels and no pixel moves by more than 2 levels of 255
/// (lib/test/widgets/spectrum_glow_test.dart). On a Radxa ZERO 3 at its 400
/// MHz GPU floor, the glow at the full resolution made the GPU miss about one
/// frame a second while music played, and at a third none.
class SpectrumGlow {
  SpectrumGlow(this._shader);

  static const double blurSigma = 12.0;
  static const double glowOpacity = 0.87;

  /// The bars the shader takes. The spectrum has 64 bands
  /// (playback/src/realtime_fft.rs); further bars are not drawn.
  static const int maxBars = 64;

  final ui.FragmentShader _shader;

  /// The image the last frame drew, kept until the next one replaces it.
  ui.Image? _lastImage;

  /// The size in pixels of the image the last frame drew.
  @visibleForTesting
  Size? get lastImageSize => _lastImage == null
      ? null
      : Size(_lastImage!.width.toDouble(), _lastImage!.height.toDouble());

  /// Draws the glow of [values] over [size], worked out in an image of
  /// [resolution] pixels for each logical pixel.
  void paint(
    Canvas canvas,
    Size size,
    List<double> values,
    Color color, {
    required double resolution,
  }) {
    if (values.isEmpty) return;

    final barWidth = size.width / values.length;
    final middle = size.height / 2;
    final count = min(values.length, maxBars);
    final alpha = color.a * glowOpacity;

    _shader
      ..setFloat(0, barWidth)
      ..setFloat(1, middle)
      ..setFloat(2, count.toDouble())
      ..setFloat(3, blurSigma)
      ..setFloat(4, color.r * alpha)
      ..setFloat(5, color.g * alpha)
      ..setFloat(6, color.b * alpha)
      ..setFloat(7, alpha);
    var tallest = 0.0;
    for (int i = 0; i < maxBars; i++) {
      final halfHeight = i < count ? (values[i] * size.height / 2).abs() : 0.0;
      _shader.setFloat(8 + i, halfHeight);
      tallest = max(tallest, halfHeight);
    }
    if (tallest == 0) return;

    // The image covers the area the blur filter drew into, grown to whole
    // pixels of the image, so that at the screen's own resolution its pixels
    // are the screen's.
    final reach = 3 * blurSigma;
    final left = ((-barWidth / 2 - reach) * resolution).floor();
    final top = ((middle - tallest - reach) * resolution).floor();
    final right = (((count - 0.5) * barWidth + reach) * resolution).ceil();
    final bottom = ((middle + tallest + reach) * resolution).ceil();
    final width = right - left;
    final height = bottom - top;
    final area = Rect.fromLTRB(
      left / resolution,
      top / resolution,
      right / resolution,
      bottom / resolution,
    );

    // The shader reads the coordinates before the scaling, so each pixel of
    // the image is the glow at the middle of the block it stands for.
    final recorder = ui.PictureRecorder();
    Canvas(recorder)
      ..scale(resolution)
      ..translate(-area.left, -area.top)
      ..drawRect(area, Paint()..shader = _shader);
    final picture = recorder.endRecording();
    final image = picture.toImageSync(width, height);
    picture.dispose();
    canvas.drawImageRect(
      image,
      Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()),
      area,
      Paint()..filterQuality = FilterQuality.low,
    );
    _lastImage?.dispose();
    _lastImage = image;
  }

  void dispose() {
    _lastImage?.dispose();
    _shader.dispose();
  }
}

class FFTPainter extends CustomPainter {
  FFTPainter(this.bars, this.color, this.glow, {required this.resolution})
    : super(repaint: bars);

  final FFTBars bars;
  final Color color;

  /// The glow, or null while its shader loads.
  final SpectrumGlow? glow;

  /// The pixels of the glow's image for each logical pixel (see
  /// SpectrumGlow.paint).
  final double resolution;

  @override
  void paint(Canvas canvas, Size size) =>
      glow?.paint(canvas, size, bars.values, color, resolution: resolution);

  @override
  bool shouldRepaint(FFTPainter oldDelegate) =>
      oldDelegate.bars != bars ||
      oldDelegate.color != color ||
      oldDelegate.glow != glow ||
      oldDelegate.resolution != resolution;
}
