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
  StreamSubscription? _clientsChangedSubscription;
  String? _deviceAlias;
  String? _fingerprint;

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

  String? get deviceAlias => _deviceAlias;
  String? get fingerprint => _fingerprint;

  BroadcastProvider() {
    _clientsChangedSubscription = TrustedClientsChanged.rustSignalStream.listen(
      (_) => fetchUsers(),
    );
    unawaited(_initializeDevice());
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
    _clientsChangedSubscription?.cancel();
    // A fetch still in flight must not notify a disposed provider.
    _pairedServersGeneration++;
    super.dispose();
  }
}
