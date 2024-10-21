// volume_provider.dart
import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../messages/playback.pb.dart';

class VolumeProvider with ChangeNotifier {
  static const String _volumeSettingsKey = 'volume_level';
  final SettingsManager _settingsManager = SettingsManager();
  late StreamSubscription<RustSignal<VolumeResponse>> _subscription;

  double _volume = 1;
  double get volume => _volume;

  VolumeProvider() {
    _initVolume();
    _subscription =
        VolumeResponse.rustSignalStream.listen(_handleVolumeResponse);
  }

  Future<void> _initVolume() async {
    double? storedVolume =
        await _settingsManager.getValue<double>(_volumeSettingsKey);
    if (storedVolume != null) {
      _updateVolume(storedVolume, notify: false, save: false);
      VolumeRequest(volume: storedVolume).sendSignalToRust();
    }
  }

  void _handleVolumeResponse(RustSignal<VolumeResponse> signal) {
    _updateVolume(signal.message.volume);
  }

  void _updateVolume(double newVolume, {bool notify = true, bool save = true}) {
    _volume = newVolume;
    if (save) _saveVolume();
    if (notify) notifyListeners();
  }

  Future<void> _saveVolume() async {
    await _settingsManager.setValue(_volumeSettingsKey, _volume);
  }

  void updateVolume(double volume) {
    _updateVolume(volume);
    VolumeRequest(volume: volume).sendSignalToRust();
  }

  @override
  void dispose() {
    super.dispose();
    _subscription.cancel();
  }
}
