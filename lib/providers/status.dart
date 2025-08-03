import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:flutter/scheduler.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/playing_item.dart';
import '../utils/settings_manager.dart';
import '../utils/theme_color_manager.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';

class PlaybackStatusState {
  String state;
  bool ready;
  double progressSeconds;
  double progressPercentage;
  String? artist;
  String? album;
  String? title;
  double duration;
  int? index;
  String? item;
  int? playbackMode;
  String? coverArtPath;
  String? libPath;

  PlaybackStatusState({
    this.state = "Stopped",
    this.ready = false,
    this.progressSeconds = 0,
    this.progressPercentage = 0,
    this.artist,
    this.album,
    this.title,
    this.duration = 0,
    this.index,
    this.item,
    this.playbackMode,
    this.coverArtPath,
    this.libPath,
  });

  PlaybackStatusState.from(PlaybackStatusState other)
      : state = other.state,
        ready = other.ready,
        progressSeconds = other.progressSeconds,
        progressPercentage = other.progressPercentage,
        artist = other.artist,
        album = other.album,
        title = other.title,
        duration = other.duration,
        index = other.index,
        item = other.item,
        playbackMode = other.playbackMode,
        coverArtPath = other.coverArtPath,
        libPath = other.libPath;

  // Update from PlaybackStatus (machine generated)
  void updateFrom(PlaybackStatus newStatus) {
    state = newStatus.state;
    ready = newStatus.ready;
    progressSeconds = newStatus.progressSeconds;
    progressPercentage = newStatus.progressPercentage;
    artist = newStatus.artist;
    album = newStatus.album;
    title = newStatus.title;
    duration = newStatus.duration;
    index = newStatus.index;
    item = newStatus.item;
    playbackMode = newStatus.playbackMode;
    coverArtPath = newStatus.coverArtPath;
    libPath = newStatus.libPath;
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other is! PlaybackStatusState) return false;

    return state == other.state &&
        ready == other.ready &&
        progressSeconds == other.progressSeconds &&
        progressPercentage == other.progressPercentage &&
        artist == other.artist &&
        album == other.album &&
        title == other.title &&
        duration == other.duration &&
        index == other.index &&
        item == other.item &&
        playbackMode == other.playbackMode &&
        coverArtPath == other.coverArtPath &&
        libPath == other.libPath;
  }

  @override
  int get hashCode => Object.hash(
        state,
        ready,
        progressSeconds,
        progressPercentage,
        artist,
        album,
        title,
        duration,
        index,
        item,
        playbackMode,
        coverArtPath,
        libPath,
      );
}

class PlaybackStatusProvider with ChangeNotifier {
  final PlaybackStatusState _playbackStatusState = PlaybackStatusState();
  PlayingItem? _playingItem;

  PlaybackStatusState get playbackStatus => _playbackStatusState;
  PlayingItem? get playingItem => _playingItem;

  late StreamSubscription<RustSignalPack<PlaybackStatus>> statusSubscription;

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

  void _updatePlaybackStatus(RustSignalPack<PlaybackStatus> signal) {
    final newStatus = signal.message;
    final previousState = PlaybackStatusState.from(_playbackStatusState);

    // Create a temporary state to compare
    final tempState = PlaybackStatusState.from(_playbackStatusState);
    tempState.updateFrom(newStatus);

    if (previousState != tempState) {
      final bool isNewTrack = _playbackStatusState.item != newStatus.item;

      _playbackStatusState.state = newStatus.state;

      if (newStatus.state != "Stopped" ||
          _playbackStatusState.libPath != newStatus.libPath) {
        _playbackStatusState.progressSeconds = newStatus.progressSeconds;
        _playbackStatusState.progressPercentage = newStatus.progressPercentage;
        _playbackStatusState.artist = newStatus.artist;
        _playbackStatusState.album = newStatus.album;
        _playbackStatusState.title = newStatus.title;
        _playbackStatusState.duration = newStatus.duration;
        _playbackStatusState.index = newStatus.index;
        _playbackStatusState.item = newStatus.item;
        _playbackStatusState.playbackMode = newStatus.playbackMode;
        _playbackStatusState.ready = newStatus.ready;
        _playbackStatusState.coverArtPath = newStatus.coverArtPath;
        _playbackStatusState.libPath = newStatus.libPath;

        final newPlayingItem =
            newStatus.item?.isEmpty == true || newStatus.item == null
                ? null
                : PlayingItem.fromString(newStatus.item!);
        _playingItem = newPlayingItem;

        if (isNewTrack) {
          if (newPlayingItem != null) {
            ThemeColorManager().handleCoverArtColorChange(newPlayingItem);
          }
          if (newStatus.index != null) {
            SettingsManager().setValue(kLastQueueIndexKey, newStatus.index!);
          }
        }
      }

      _scheduleNotification();
    }
  }

  bool get notReady {
    return _playbackStatusState.ready == false;
  }
}
