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

    SetRealtimeFFTEnabledRequest(enabled: shouldCalculateFFT)
        .sendSignalToRust();
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
  final radius = 12.0;
  final _bars = FFTBars();
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

  @override
  dispose() {
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
            child: Opacity(
              opacity: 0.87,
              child: ImageFiltered(
                imageFilter:
                    ui.ImageFilter.blur(sigmaX: radius, sigmaY: radius),
                child: CustomPaint(
                  painter: FFTPainter(_bars, color),
                ),
              ),
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

class FFTPainter extends CustomPainter {
  final FFTBars bars;
  final Color color;

  FFTPainter(this.bars, this.color) : super(repaint: bars);

  @override
  void paint(Canvas canvas, Size size) {
    final fftValues = bars.values;
    final paint = Paint()
      ..color = color
      ..strokeWidth = size.width / fftValues.length;

    final midY = size.height / 2;

    for (int i = 0; i < fftValues.length; i++) {
      final x = i * (size.width / fftValues.length);
      final y = fftValues[i] * size.height / 2;
      canvas.drawLine(Offset(x, midY - y), Offset(x, midY + y), paint);
    }
  }

  @override
  bool shouldRepaint(FFTPainter oldDelegate) =>
      oldDelegate.bars != bars || oldDelegate.color != color;
}
