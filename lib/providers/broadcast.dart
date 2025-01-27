import 'dart:async';
import 'package:flutter/foundation.dart';

import '../messages/all.dart';
import '../utils/settings_manager.dart';

final SettingsManager _settingsManager = SettingsManager();

class BroadcastProvider extends ChangeNotifier {
  static const String _configKey = 'broadcast_config';
  static const int _defaultDuration = 300;

  Timer? _countdownTimer;
  int _remainingSeconds = 0;
  bool _isBroadcasting = false;
  String? _errorMessage;

  int get remainingSeconds => _remainingSeconds;
  bool get isBroadcasting => _isBroadcasting;
  String? get errorMessage => _errorMessage;

  BroadcastProvider() {
    _loadConfiguration();
  }

  Future<void> _loadConfiguration() async {
    try {
      final config =
          await _settingsManager.getValue<Map<String, dynamic>>(_configKey);
      if (config != null) {
        _remainingSeconds = config['last_duration'] ?? _defaultDuration;
      } else {
        _remainingSeconds = _defaultDuration;
      }
    } catch (e) {
      _remainingSeconds = _defaultDuration;
    }
  }

  Future<void> _saveConfiguration() async {
    await _settingsManager.setValue(_configKey, {
      'last_duration': _remainingSeconds,
    });
  }

  Future<void> startBroadcast([int? customDuration]) async {
    if (_isBroadcasting) return;

    final duration = customDuration ?? _remainingSeconds;
    _isBroadcasting = true;
    _errorMessage = null;
    _remainingSeconds = duration;

    notifyListeners();
    _saveConfiguration();

    StartBroadcastRequest(durationSeconds: duration).sendSignalToRust();

    _startCountdownTimer(duration);
  }

  Future<void> stopBroadcast() async {
    if (!_isBroadcasting) return;

    StopBroadcastRequest().sendSignalToRust();
    _countdownTimer?.cancel();
    _isBroadcasting = false;
    notifyListeners();
  }

  void _startCountdownTimer(int totalSeconds) {
    var startTime = DateTime.now().millisecondsSinceEpoch;

    _countdownTimer = Timer.periodic(const Duration(seconds: 1), (timer) {
      final elapsed =
          (DateTime.now().millisecondsSinceEpoch - startTime) ~/ 1000;
      _remainingSeconds = totalSeconds - elapsed;

      if (_remainingSeconds <= 0) {
        timer.cancel();
        _isBroadcasting = false;
        _remainingSeconds = _defaultDuration;
      }

      notifyListeners();
    });
  }

  @override
  void dispose() {
    _countdownTimer?.cancel();
    super.dispose();
  }
}
