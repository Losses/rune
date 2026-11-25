import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../constants/configurations.dart';
import '../utils/kept_clear.dart';
import '../utils/settings_manager.dart';
import 'responsive_providers.dart';

/// The interface scale that follows the system text size: 1.0 up to the
/// default size, otherwise the zoom nearest to [textScale] that covers a whole
/// number of device pixels per logical pixel.
double textSizeScaleFactor(double textScale, double devicePixelRatio) {
  if (textScale <= 1 || devicePixelRatio <= 0) return 1.0;
  return (textScale * devicePixelRatio).round() / devicePixelRatio;
}

/// The zoom that follows the system text size ([textSizeScaleFactor]), held
/// at the layout floor: where the nearest zoom would turn a phone's layout
/// ([isPhoneLayoutSize]) into a strip or the dock, the largest whole-pixel
/// zoom that keeps one, or 1 where none does. [area] is the window less the
/// insets kept clear of, before any zoom.
///
/// The text size itself has no cap: what the zoom leaves of it is scaled on
/// the text alone.
double followedScaleFactor(
  double textScale,
  double devicePixelRatio,
  Size area,
) {
  final zoom = textSizeScaleFactor(textScale, devicePixelRatio);
  for (
    var pixels = (zoom * devicePixelRatio).round();
    pixels > devicePixelRatio;
    pixels--
  ) {
    final step = pixels / devicePixelRatio;
    if (isPhoneLayoutSize(area / step)) return step;
  }
  return math.min(zoom, 1.0);
}

/// The interface scale: a percentage the user picks, or, where it is
/// supported, a zoom that follows the system text size.
class AppScaleProvider extends ChangeNotifier with WidgetsBindingObserver {
  AppScaleProvider({
    Future<Object?> Function()? read,
    Future<void> Function(int)? write,
    this.supportsFollowTextSize = false,
  }) : _read =
           read ??
           (() => SettingsManager().getValue<Object>(kInterfaceScaleKey)),
       _write =
           write ??
           ((value) =>
               SettingsManager().setValue<int>(kInterfaceScaleKey, value)) {
    if (supportsFollowTextSize) {
      _percentage = followTextSize;
      _followedFactor = _computeFollowedFactor();
      WidgetsBinding.instance.addObserver(this);
    }
  }

  /// The stored choice that follows the system text size
  /// ([followedScaleFactor]) in place of a percentage.
  static const followTextSize = 0;

  static const supportedPercentages = <int>[75, 100, 125, 150, 175, 200];

  /// Whether [followTextSize] can be chosen, which then is the default.
  final bool supportsFollowTextSize;
  final Future<Object?> Function() _read;
  final Future<void> Function(int) _write;
  int _percentage = 100;
  double _followedFactor = 1.0;
  Future<void> _writes = Future.value();
  int _revision = 0;
  bool _disposed = false;

  /// The choice without a stored one: following the text size where that is
  /// supported, else 100%.
  int get defaultPercentage => supportsFollowTextSize ? followTextSize : 100;

  int get percentage => _percentage;

  /// Whether the scale follows the system text size rather than a
  /// percentage.
  bool get followingTextSize => _percentage == followTextSize;

  double get scaleFactor =>
      followingTextSize ? _followedFactor : _percentage / 100;

  double _computeFollowedFactor() {
    final dispatcher = WidgetsBinding.instance.platformDispatcher;
    final views = dispatcher.views;
    if (views.isEmpty) return 1.0;
    final view = views.first;
    final ratio = view.devicePixelRatio;
    if (ratio <= 0) return 1.0;
    // The area the layout is chosen by, as ScreenSizeProvider has it.
    final window = view.physicalSize / ratio;
    final keptClear = keptClearOf(
      EdgeInsets.fromViewPadding(view.viewPadding, ratio),
    );
    return followedScaleFactor(
      dispatcher.textScaleFactor,
      ratio,
      Size(
        math.max(0.0, window.width - keptClear.horizontal),
        math.max(0.0, window.height - keptClear.vertical),
      ),
    );
  }

  void _refreshFollowedFactor() {
    final factor = _computeFollowedFactor();
    if (factor == _followedFactor) return;
    _followedFactor = factor;
    if (followingTextSize) notifyListeners();
  }

  @override
  void didChangeTextScaleFactor() => _refreshFollowedFactor();

  @override
  void didChangeMetrics() => _refreshFollowedFactor();

  Future<void> initialize() async {
    final revision = _revision;
    final value = await _read();
    if (_disposed || revision != _revision) return;
    final validated = value is int && _isSupported(value)
        ? value
        : defaultPercentage;
    if (_percentage != validated) {
      _percentage = validated;
      notifyListeners();
    }
  }

  bool _isSupported(int value) =>
      supportedPercentages.contains(value) ||
      (supportsFollowTextSize && value == followTextSize);

  @override
  void dispose() {
    if (supportsFollowTextSize) WidgetsBinding.instance.removeObserver(this);
    _disposed = true;
    super.dispose();
  }

  Future<void> setPercentage(int value) {
    if (!_isSupported(value)) {
      throw ArgumentError.value(value, 'value', 'Unsupported interface scale');
    }
    _revision++;
    // A failed write must not prevent later choices from being saved.
    final operation = _writes.catchError((Object _) {}).then((_) async {
      if (_percentage == value) return;
      await _write(value);
      if (_disposed) return;
      _percentage = value;
      notifyListeners();
    });
    _writes = operation;
    return operation;
  }
}
