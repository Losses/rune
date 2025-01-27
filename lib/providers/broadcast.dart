import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import 'package:uuid/uuid.dart';

import '../messages/all.dart';
import '../utils/settings_manager.dart';

final SettingsManager _settingsManager = SettingsManager();

class BroadcastProvider extends ChangeNotifier {
  static const String _deviceAliasKey = 'device_alias';
  static const String _fingerprintKey = 'device_fingerprint';
  static const int _defaultDuration = 300;

  Timer? _countdownTimer;
  int _remainingSeconds = 0;
  bool _isBroadcasting = false;
  String? _errorMessage;
  String? _deviceAlias;
  String? _fingerprint;

  int get remainingSeconds => _remainingSeconds;
  bool get isBroadcasting => _isBroadcasting;
  String? get errorMessage => _errorMessage;
  String? get deviceAlias => _deviceAlias;
  String? get fingerprint => _fingerprint;

  BroadcastProvider() {
    _initializeDevice();
  }

  Future<void> _initializeDevice() async {
    await Future.wait([
      _initializeDeviceAlias(),
      _initializeFingerprint(),
    ]);
  }

  String _generateRandomAlias() {
    const chars =
        'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    final random = Random();
    return 'R-${List.generate(8, (index) => chars[random.nextInt(chars.length)]).join()}';
  }

  Future<void> _initializeDeviceAlias() async {
    _deviceAlias = await _settingsManager.getValue<String>(_deviceAliasKey);
    if (_deviceAlias == null) {
      _deviceAlias = _generateRandomAlias();
      await _settingsManager.setValue(_deviceAliasKey, _deviceAlias);
    }
  }

  Future<void> _initializeFingerprint() async {
    _fingerprint = await _settingsManager.getValue<String>(_fingerprintKey);
    if (_fingerprint == null) {
      _fingerprint = const Uuid().v4();
      await _settingsManager.setValue(_fingerprintKey, _fingerprint);
    }
  }

  Future<void> updateDeviceAlias(String newAlias) async {
    await _settingsManager.setValue(_deviceAliasKey, newAlias);
    _deviceAlias = newAlias;
    notifyListeners();
  }

  Future<void> startBroadcast([int? customDuration]) async {
    if (_isBroadcasting) return;

    final duration = customDuration ?? _remainingSeconds;
    _isBroadcasting = true;
    _errorMessage = null;
    _remainingSeconds = duration;

    notifyListeners();

    StartBroadcastRequest(
      durationSeconds: duration,
      alias: _deviceAlias,
      fingerprint: _fingerprint,
    ).sendSignalToRust();

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
