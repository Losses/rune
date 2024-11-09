import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../messages/playback.pb.dart';
import '../utils/theme_color_manager.dart';

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
    final newStatus = signal.message;
    if (_playbackStatus == null ||
        !_isPlaybackStatusEqual(_playbackStatus!, newStatus)) {
      final bool isNewTrack = _playbackStatus?.id != newStatus.id;
      _playbackStatus = newStatus;

      if (isNewTrack && newStatus.state != "Stopped") {
        ThemeColorManager().handleCoverArtColorChange(newStatus.id);
      }

      notifyListeners();
    }
  }

  bool _isPlaybackStatusEqual(
      PlaybackStatus oldStatus, PlaybackStatus newStatus) {
    return oldStatus.state == newStatus.state &&
        oldStatus.progressSeconds == newStatus.progressSeconds &&
        oldStatus.progressPercentage == newStatus.progressPercentage &&
        oldStatus.artist == newStatus.artist &&
        oldStatus.album == newStatus.album &&
        oldStatus.title == newStatus.title &&
        oldStatus.duration == newStatus.duration &&
        oldStatus.index == newStatus.index &&
        oldStatus.id == newStatus.id &&
        oldStatus.playbackMode == newStatus.playbackMode &&
        oldStatus.ready == newStatus.ready &&
        oldStatus.coverArtPath == newStatus.coverArtPath;
  }

  bool get notReady {
    return _playbackStatus?.ready == null || _playbackStatus!.ready == false;
  }
}
