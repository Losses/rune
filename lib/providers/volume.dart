import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../messages/playback.pb.dart';

class VolumeProvider with ChangeNotifier {
  double _volume = 1;

  double get volume => _volume;

  late StreamSubscription<RustSignal<VolumeResponse>> subscription;

  VolumeProvider() {
    subscription = VolumeResponse.rustSignalStream.listen(_updateVolume);
  }

  void _updateVolume(RustSignal<VolumeResponse> signal) {
    _volume = signal.message.volume;
    notifyListeners();
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void updateVolume(double volume) {
    VolumeRequest(volume: volume).sendSignalToRust();
  }
}
