import 'package:flutter/foundation.dart';

import '../constants/configurations.dart';
import '../utils/settings_manager.dart';

class AppScaleProvider extends ChangeNotifier {
  AppScaleProvider({
    Future<Object?> Function()? read,
    Future<void> Function(int)? write,
  }) : _read =
           read ??
           (() => SettingsManager().getValue<Object>(kInterfaceScaleKey)),
       _write =
           write ??
           ((value) =>
               SettingsManager().setValue<int>(kInterfaceScaleKey, value));

  static const supportedPercentages = <int>[75, 100, 125, 150, 175, 200];
  final Future<Object?> Function() _read;
  final Future<void> Function(int) _write;
  int _percentage = 100;
  Future<void> _writes = Future.value();
  int _revision = 0;
  bool _disposed = false;

  int get percentage => _percentage;
  double get scaleFactor => _percentage / 100;

  Future<void> initialize() async {
    final revision = _revision;
    final value = await _read();
    if (_disposed || revision != _revision) return;
    final validated = value is int && supportedPercentages.contains(value)
        ? value
        : 100;
    if (_percentage != validated) {
      _percentage = validated;
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  Future<void> setPercentage(int value) {
    if (!supportedPercentages.contains(value)) {
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
