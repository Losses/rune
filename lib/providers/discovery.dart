import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:flutter/foundation.dart';

import '../utils/settings_manager.dart';
import '../messages/all.dart';
import '../constants/configurations.dart';

class DiscoveredDevice {
  final String alias;
  final String deviceModel;
  final String deviceType;
  final String fingerprint;
  final DateTime lastSeen;
  final List<String> ips;

  DiscoveredDevice({
    required this.alias,
    required this.deviceModel,
    required this.deviceType,
    required this.fingerprint,
    required this.lastSeen,
    required this.ips,
  });

  factory DiscoveredDevice.fromMessage(DiscoveredDeviceMessage message) {
    return DiscoveredDevice(
      alias: message.alias,
      deviceModel: message.deviceModel,
      deviceType: message.deviceType,
      fingerprint: message.fingerprint,
      lastSeen: DateTime.fromMillisecondsSinceEpoch(
        message.lastSeenUnixEpoch.toInt() * 1000,
      ),
      ips: message.ips.toList(),
    );
  }
}

class DeviceListenerProvider with ChangeNotifier {
  final Map<String, DiscoveredDevice> _devices = {};
  final SettingsManager _settingsManager = SettingsManager();
  StreamSubscription<RustSignal<DiscoveredDeviceMessage>>? _subscription;
  bool _isListening = false;
  String? _error;

  Map<String, DiscoveredDevice> get devices => Map.unmodifiable(_devices);
  bool get isListening => _isListening;
  String? get error => _error;

  Future<void> startListening() async {
    if (_isListening) return;

    try {
      final alias = await _settingsManager.getValue<String>(kDeviceAliasKey);
      final fingerprint =
          await _settingsManager.getValue<String>(kFingerprintKey);

      if (alias == null || fingerprint == null) {
        throw Exception('Device identity not initialized');
      }

      _subscription?.cancel();
      _subscription =
          DiscoveredDeviceMessage.rustSignalStream.listen(_handleDeviceFound);

      StartListeningRequest(alias: alias, fingerprint: fingerprint)
          .sendSignalToRust();

      _isListening = true;
      _error = null;
      notifyListeners();
    } catch (e) {
      _error = e.toString();
      notifyListeners();
    }
  }

  Future<void> stopListening() async {
    if (!_isListening) return;

    StopListeningRequest().sendSignalToRust();
    await _subscription?.cancel();

    _devices.clear();
    _isListening = false;
    _error = null;
    notifyListeners();
  }

  void _handleDeviceFound(RustSignal<DiscoveredDeviceMessage> event) {
    final device = DiscoveredDevice.fromMessage(event.message);
    _devices[device.fingerprint] = device;
    notifyListeners();
  }

  @override
  void dispose() {
    _subscription?.cancel();
    super.dispose();
  }
}
