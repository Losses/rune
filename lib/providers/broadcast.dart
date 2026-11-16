import 'dart:math';
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../utils/settings_manager.dart';
import '../utils/api/get_ssl_certificate_fingerprint.dart';
import '../utils/api/request_failure.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';

final SettingsManager _settingsManager = SettingsManager();

class BroadcastProvider extends ChangeNotifier {
  static const int _defaultDuration = 300;

  Timer? _countdownTimer;
  StreamSubscription? _clientsChangedSubscription;
  int _remainingSeconds = 0;
  bool _isBroadcasting = false;
  String? _errorMessage;
  String? _deviceAlias;
  String? _fingerprint;

  bool _isServerRunning = false;
  String? _interface;
  late Completer<void> _initializationCompleter;

  List<ClientSummary> _users = [];
  List<ClientSummary> get users => _users;

  int get remainingSeconds => _remainingSeconds;
  bool get isBroadcasting => _isBroadcasting;
  String? get errorMessage => _errorMessage;
  String? get deviceAlias => _deviceAlias;
  String? get fingerprint => _fingerprint;
  bool get isServerRunning => _isServerRunning;
  String? get interface => _interface;

  BroadcastProvider() {
    _clientsChangedSubscription = TrustedClientsChanged.rustSignalStream.listen(
      (_) => fetchUsers(),
    );
    _initializationCompleter = Completer<void>();
    _initializeDevice().then((_) {
      _initializationCompleter.complete();
    });
  }

  Future<void> _initializeDevice() async {
    await _initializeDeviceAlias(); // Ensure device alias is initialized first
    await _initializeFingerprint(); // Then initialize fingerprint (depends on device alias)
  }

  Future<void> _initializeFingerprint() async {
    _fingerprint = await getSSLCertificateFingerprint();
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
    if (!_isServerRunning) {
      _errorMessage = 'Server must be running to broadcast';
      notifyListeners();
      return;
    }

    final duration = customDuration ?? _defaultDuration;
    _isBroadcasting = true;
    _errorMessage = null;
    _remainingSeconds = duration;

    notifyListeners();

    StartBroadcastRequest(
      durationSeconds: duration,
      alias: _deviceAlias ?? "",
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

  /// Start the server. A failure is left to the global handler.
  Future<void> startServer(String interface) async {
    if (_isServerRunning) return;
    await _initializationCompleter.future;

    _interface = interface;
    notifyListeners();

    try {
      await awaitResponse(
        'StartServerRequest',
        StartServerResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => StartServerRequest(
          interface: interface,
          alias: _deviceAlias!,
          callId: callId,
        ).sendSignalToRust(),
      );
    } on RequestFailure {
      stopBroadcast();
      rethrow;
    }
    _isServerRunning = true;
    notifyListeners();
  }

  /// Stop the server. A failure is left to the global handler.
  void stopServer() {
    if (!_isServerRunning) return;
    unawaited(_stopServer());
  }

  Future<void> _stopServer() async {
    await awaitResponse(
      'StopServerRequest',
      StopServerResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => StopServerRequest(callId: callId).sendSignalToRust(),
    );
    _isServerRunning = false;
    stopBroadcast();
    notifyListeners();
  }

  /// Reload the trusted devices. A failure is left to the global handler.
  void fetchUsers() {
    unawaited(_fetchUsers());
  }

  Future<void> _fetchUsers() async {
    final response = await awaitResponse(
      'ListClientsRequest',
      ListClientsResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => ListClientsRequest(callId: callId).sendSignalToRust(),
    );
    _users = response.message.users;
    notifyListeners();
  }

  @override
  void dispose() {
    _countdownTimer?.cancel();
    _clientsChangedSubscription?.cancel();
    super.dispose();
  }
}
