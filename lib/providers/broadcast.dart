import 'dart:io';
import 'dart:math';
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../constants/configurations.dart';
import '../messages/all.dart';
import '../utils/settings_manager.dart';
import '../utils/ssl.dart';

final SettingsManager _settingsManager = SettingsManager();

class BroadcastProvider extends ChangeNotifier {
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
    await _initializeDeviceAlias(); // Ensure device alias is initialized first
    await _initializeFingerprint(); // Then initialize fingerprint (depends on device alias)
  }

  Future<void> _initializeFingerprint() async {
    // Get settings path and file objects
    final settingsPath = await getSettingsPath();
    final certFile = File('$settingsPath/certificate.pem');
    final keyFile = File('$settingsPath/private_key.pem');

    // Check if certificate files and fingerprint exist
    final certExists = await certFile.exists();
    final keyExists = await keyFile.exists();
    _fingerprint = await _settingsManager.getValue<String>(kFingerprintKey);

    // Conditions requiring regeneration of the certificate
    if (_fingerprint == null || !certExists || !keyExists) {
      final certResult = await generateSelfSignedCertificate(
        commonName: _deviceAlias!,
        organization: 'Rune Device',
        country: 'NET',
        validityDays: 3650, // 10-year validity
      );

      // Save certificate and private key
      await certFile.writeAsString(certResult.certificate);
      await keyFile.writeAsString(certResult.privateKey);

      // Update fingerprint information
      _fingerprint = certResult.publicKeyFingerprint;
      await _settingsManager.setValue(kFingerprintKey, _fingerprint);
    }

    notifyListeners();
  }

  String _generateRandomAlias() {
    const chars =
        'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    final random = Random();
    return 'R-${List.generate(8, (index) => chars[random.nextInt(chars.length)]).join()}';
  }

  Future<void> _initializeDeviceAlias() async {
    _deviceAlias = await _settingsManager.getValue<String>(kDeviceAliasKey);
    if (_deviceAlias == null) {
      _deviceAlias = _generateRandomAlias();
      await _settingsManager.setValue(kDeviceAliasKey, _deviceAlias);
    }
  }

  Future<void> updateDeviceAlias(String newAlias) async {
    await _settingsManager.setValue(kDeviceAliasKey, newAlias);
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
