import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../messages/playback.pb.dart';

class PlaybackStatusProvider with ChangeNotifier {
  PlaybackStatus? _playbackStatus;

  PlaybackStatus? get playbackStatus => _playbackStatus;

  late StreamSubscription<RustSignal<PlaybackStatus>> subscription;

  PlaybackStatusProvider() {
    subscription =
        PlaybackStatus.rustSignalStream.listen(_updatePlaybackStatus);
  }

  @override
  void dispose() {
    super.dispose();
    subscription.cancel();
  }

  void _updatePlaybackStatus(RustSignal<PlaybackStatus> signal) {
    _playbackStatus = signal.message;
    notifyListeners();
  }
}
