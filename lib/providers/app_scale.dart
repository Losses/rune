import 'package:flutter/widgets.dart';

import '../constants/configurations.dart';
import '../utils/settings_manager.dart';

/// The interface scale that follows the system text size: 1.0 up to the
/// default size, otherwise the zoom nearest to [textScale] that covers a whole
/// number of device pixels per logical pixel.
double textSizeScaleFactor(double textScale, double devicePixelRatio) {
  if (textScale <= 1 || devicePixelRatio <= 0) return 1.0;
  return (textScale * devicePixelRatio).round() / devicePixelRatio;
}

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

  static const followTextSize = 0;
  static const supportedPercentages = <int>[75, 100, 125, 150, 175, 200];
  final bool supportsFollowTextSize;
  final Future<Object?> Function() _read;
  final Future<void> Function(int) _write;
  int _percentage = 100;
  double _followedFactor = 1.0;
  Future<void> _writes = Future.value();
  int _revision = 0;
  bool _disposed = false;

  int get defaultPercentage => supportsFollowTextSize ? followTextSize : 100;
  int get percentage => _percentage;
  bool get followingTextSize => _percentage == followTextSize;
  double get scaleFactor =>
      followingTextSize ? _followedFactor : _percentage / 100;

  double _computeFollowedFactor() {
    final dispatcher = WidgetsBinding.instance.platformDispatcher;
    final views = dispatcher.views;
    if (views.isEmpty) return 1.0;
    return textSizeScaleFactor(
      dispatcher.textScaleFactor,
      views.first.devicePixelRatio,
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
