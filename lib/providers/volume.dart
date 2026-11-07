// volume_provider.dart
import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../bindings/bindings.dart';

class VolumeProvider with ChangeNotifier {
  static const String _volumeSettingsKey = 'volume_level';

  final Future<double?> Function() _readStoredVolume;
  final Future<void> Function(double volume) _writeStoredVolume;
  final void Function(VolumeRequest request) _sendVolumeRequest;
  late StreamSubscription<RustSignalPack<VolumeResponse>> _subscription;

  double _volume = 1;
  double get volume => _volume;

  Timer? _debounceTimer;
  int _userVolumeGeneration = 0;

  VolumeProvider({
    Future<double?> Function()? readStoredVolume,
    Future<void> Function(double volume)? writeStoredVolume,
    void Function(VolumeRequest request)? sendVolumeRequest,
  }) : _readStoredVolume = readStoredVolume ?? _readVolumeFromSettings,
       _writeStoredVolume = writeStoredVolume ?? _writeVolumeToSettings,
       _sendVolumeRequest = sendVolumeRequest ?? _sendVolumeToRust {
    _initVolume();
    _subscription = VolumeResponse.rustSignalStream.listen(
      _handleVolumeResponse,
    );
  }

  Future<void> _initVolume() async {
    final generation = _userVolumeGeneration;
    final storedVolume = await _readStoredVolume();
    if (generation != _userVolumeGeneration) return;
    if (storedVolume != null) {
      _updateVolume(storedVolume, notify: false, save: false);
      _sendVolumeRequest(VolumeRequest(volume: storedVolume));
    }
  }

  static Future<double?> _readVolumeFromSettings() =>
      SettingsManager().getValue<double>(_volumeSettingsKey);

  static Future<void> _writeVolumeToSettings(double volume) =>
      SettingsManager().setValue(_volumeSettingsKey, volume);

  static void _sendVolumeToRust(VolumeRequest request) {
    request.sendSignalToRust();
  }

  void _handleVolumeResponse(RustSignalPack<VolumeResponse> signal) {
    _updateVolume(signal.message.volume);
  }

  void _updateVolume(double newVolume, {bool notify = true, bool save = true}) {
    _volume = newVolume;
    if (save) _saveVolume();
    if (notify) notifyListeners();
  }

  Future<void> _saveVolume() async {
    await _writeStoredVolume(_volume);
  }

  void updateVolume(double volume) {
    _userVolumeGeneration++;
    _updateVolume(volume);

    // Cancel the previous timer if it exists
    _debounceTimer?.cancel();

    // Start a new timer
    _debounceTimer = Timer(Duration(milliseconds: 200), () {
      _sendVolumeRequest(VolumeRequest(volume: volume));
    });
  }

  @override
  void dispose() {
    _subscription.cancel();
    _debounceTimer?.cancel();
    super.dispose();
  }
}
