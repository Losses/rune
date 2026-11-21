import 'dart:math';
import 'dart:async';

import 'package:flutter/foundation.dart';

import '../utils/settings_manager.dart';
import '../utils/rune_log.dart';
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
  bool _isUpdatingBroadcast = false;
  String? _errorMessage;
  String? _deviceAlias;
  String? _fingerprint;

  bool _isServerRunning = false;
  String? _interface;
  late Completer<void> _initializationCompleter;
  late final Future<void> _deviceAliasReady = _initializeDeviceAlias();

  List<ClientSummary> _users = [];
  List<ClientSummary> get users => _users;

  List<ClientSummary> _pairedServers = const [];
  int _pairedServersGeneration = 0;
  bool _isLoadingPairedServers = false;

  /// The devices this device trusts as servers, with the status each reports
  /// for this device.
  List<ClientSummary> get pairedServers => _pairedServers;
  bool get isLoadingPairedServers => _isLoadingPairedServers;

  int get remainingSeconds => _remainingSeconds;
  bool get isBroadcasting => _isBroadcasting;

  /// Whether a broadcast command or server stop is awaiting a reply.
  bool get isUpdatingBroadcast => _isUpdatingBroadcast;
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
    await _deviceAliasReady;
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
    try {
      _deviceAlias = await _settingsManager.getValue<String>(kDeviceAliasKey);
      if (_deviceAlias == null) {
        _deviceAlias = _generateRandomAlias();
        await _settingsManager.setValue(kDeviceAliasKey, _deviceAlias);
      }
    } on Exception catch (error) {
      _deviceAlias ??= _generateRandomAlias();
      warn$('Could not initialize the stored device name: $error');
    }
  }

  Future<void> updateDeviceAlias(String newAlias) async {
    if (newAlias.trim().isEmpty) return;

    await _deviceAliasReady;
    await _settingsManager.setValue(kDeviceAliasKey, newAlias);
    _deviceAlias = newAlias;
    notifyListeners();
  }

  Future<void> startBroadcast([int? customDuration]) async {
    if (_isBroadcasting || _isUpdatingBroadcast) return;
    if (!_isServerRunning) {
      _errorMessage = 'Server must be running to broadcast';
      notifyListeners();
      return;
    }

    final duration = customDuration ?? _defaultDuration;
    await _updateBroadcast(() async {
      try {
        await awaitResponse(
          'StartBroadcastRequest',
          StartBroadcastResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) => StartBroadcastRequest(
            durationSeconds: duration,
            alias: _deviceAlias ?? '',
            callId: callId,
          ).sendSignalToRust(),
        );
      } on RequestFailure catch (error) {
        if (error.cause
            is! RequestFailureCauseIndefiniteBroadcastAlreadyRunning) {
          rethrow;
        }
        // The server already started announcements without this countdown.
        _isBroadcasting = true;
        return;
      }

      _isBroadcasting = duration > 0;
      _remainingSeconds = duration;
      if (_isBroadcasting) _startCountdownTimer(duration);
    });
  }

  Future<void> stopBroadcast() async {
    if (!_isBroadcasting || _isUpdatingBroadcast) return;

    await _updateBroadcast(() async {
      await _requestBroadcastStop();
      _clearBroadcast();
    });
  }

  Future<void> _requestBroadcastStop() async {
    await awaitResponse(
      'StopBroadcastRequest',
      StopBroadcastResponse.rustSignalStream,
      responseCallId: (response) => response.message.callId?.toInt(),
      send: (callId) => StopBroadcastRequest(callId: callId).sendSignalToRust(),
    );
  }

  Future<void> _updateBroadcast(Future<void> Function() update) async {
    _isUpdatingBroadcast = true;
    _errorMessage = null;
    notifyListeners();
    try {
      await update();
    } finally {
      _isUpdatingBroadcast = false;
      notifyListeners();
    }
  }

  void _clearBroadcast() {
    _countdownTimer?.cancel();
    _isBroadcasting = false;
    _remainingSeconds = 0;
  }

  void _startCountdownTimer(int totalSeconds) {
    _countdownTimer = Timer.periodic(const Duration(seconds: 1), (timer) {
      _remainingSeconds = totalSeconds - timer.tick;

      if (_remainingSeconds <= 0) _clearBroadcast();

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
    if (!_isServerRunning || _isUpdatingBroadcast) return;
    final wasBroadcasting = _isBroadcasting;
    unawaited(
      _updateBroadcast(() async {
        await awaitResponse(
          'StopServerRequest',
          StopServerResponse.rustSignalStream,
          responseCallId: (response) => response.message.callId?.toInt(),
          send: (callId) =>
              StopServerRequest(callId: callId).sendSignalToRust(),
        );
        _isServerRunning = false;
        if (wasBroadcasting) await _requestBroadcastStop();
        _clearBroadcast();
      }),
    );
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

  /// Reload the paired servers. A failure keeps the previous list.
  void fetchPairedServers() {
    unawaited(_fetchPairedServers());
  }

  Future<void> _fetchPairedServers() async {
    // The hub probes each server over the network, so an older, slower reply
    // must not replace a newer one.
    final generation = ++_pairedServersGeneration;
    _isLoadingPairedServers = true;
    notifyListeners();
    try {
      final response = await awaitResponse(
        'ListPairedServersRequest',
        ListPairedServersResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            ListPairedServersRequest(callId: callId).sendSignalToRust(),
      );
      if (generation == _pairedServersGeneration) {
        _pairedServers = response.message.servers;
      }
    } catch (error) {
      warn$('Could not list the paired servers: $error');
    } finally {
      if (generation == _pairedServersGeneration) {
        _isLoadingPairedServers = false;
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    _countdownTimer?.cancel();
    _clientsChangedSubscription?.cancel();
    // A fetch still in flight must not notify a disposed provider.
    _pairedServersGeneration++;
    super.dispose();
  }
}
