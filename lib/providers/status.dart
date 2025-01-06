import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/playing_item.dart';
import '../utils/settings_manager.dart';
import '../utils/theme_color_manager.dart';
import '../messages/all.dart';
import '../constants/configurations.dart';

class PlaybackStatusProvider with ChangeNotifier {
  final PlaybackStatus _playbackStatus =
      PlaybackStatus(state: "Stopped", ready: false);
  PlayingItem? _playingItem;

  PlaybackStatus get playbackStatus => _playbackStatus;
  PlayingItem? get playingItem => _playingItem;

  late StreamSubscription<RustSignal<PlaybackStatus>> statusSubscription;

  bool _hasPendingNotification = false;

  PlaybackStatusProvider() {
    statusSubscription =
        PlaybackStatus.rustSignalStream.listen(_updatePlaybackStatus);
  }

  @override
  void dispose() {
    super.dispose();
    statusSubscription.cancel();
  }

  void _scheduleNotification() {
    if (!_hasPendingNotification) {
      _hasPendingNotification = true;
      SchedulerBinding.instance.addPostFrameCallback((_) {
        notifyListeners();
        _hasPendingNotification = false;
      });
    }
  }

  void _updatePlaybackStatus(RustSignal<PlaybackStatus> signal) {
    final newStatus = signal.message;
    if (!_isPlaybackStatusEqual(_playbackStatus, newStatus)) {
      final bool isNewTrack = _playbackStatus.item != newStatus.item;

      _playbackStatus.state = newStatus.state;

      if (newStatus.state != "Stopped" || _playbackStatus.libPath != newStatus.libPath) {
        _playbackStatus.progressSeconds = newStatus.progressSeconds;
        _playbackStatus.progressPercentage = newStatus.progressPercentage;
        _playbackStatus.artist = newStatus.artist;
        _playbackStatus.album = newStatus.album;
        _playbackStatus.title = newStatus.title;
        _playbackStatus.duration = newStatus.duration;
        _playbackStatus.index = newStatus.index;
        _playbackStatus.item = newStatus.item;
        _playbackStatus.playbackMode = newStatus.playbackMode;
        _playbackStatus.ready = newStatus.ready;
        _playbackStatus.coverArtPath = newStatus.coverArtPath;
        _playbackStatus.libPath = newStatus.libPath;
        final newPlayingItem = newStatus.item.isEmpty
            ? null
            : PlayingItem.fromString(newStatus.item);
        _playingItem = newPlayingItem;

        if (isNewTrack) {
          if (newPlayingItem != null) {
            ThemeColorManager().handleCoverArtColorChange(newPlayingItem);
          }
          SettingsManager().setValue(kLastQueueIndexKey, newStatus.index);
        }
      }

      _scheduleNotification();
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
        oldStatus.item == newStatus.item &&
        oldStatus.playbackMode == newStatus.playbackMode &&
        oldStatus.ready == newStatus.ready &&
        oldStatus.coverArtPath == newStatus.coverArtPath;
  }

  bool get notReady {
    return playbackStatus.ready == false;
  }
}
