import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:get_storage/get_storage.dart';

import '../messages/playback.pb.dart';

class VolumeProvider with ChangeNotifier {
  static const String storageKey = 'volume_level';
  final GetStorage _storage = GetStorage();
  late StreamSubscription<RustSignal<VolumeResponse>> _subscription;

  double _volume = 1;
  double get volume => _volume;

  VolumeProvider() {
    _initVolume();
    _subscription =
        VolumeResponse.rustSignalStream.listen(_handleVolumeResponse);
  }

  Future<void> _initVolume() async {
    await GetStorage.init();
    double? storedVolume = _storage.read<double>(storageKey);
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

  void _saveVolume() {
    _storage.write(storageKey, _volume);
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
