enum PlaybackMode {
  sequential,
  repeatOne,
  repeatAll,
  shuffle,
}

extension PlaybackModeExtension on PlaybackMode {
  int toValue() {
    switch (this) {
      case PlaybackMode.sequential:
        return 0;
      case PlaybackMode.repeatOne:
        return 1;
      case PlaybackMode.repeatAll:
        return 2;
      case PlaybackMode.shuffle:
        return 3;
    }
  }

  static PlaybackMode fromValue(int value) {
    switch (value) {
      case 0:
        return PlaybackMode.sequential;
      case 1:
        return PlaybackMode.repeatOne;
      case 2:
        return PlaybackMode.repeatAll;
      case 3:
        return PlaybackMode.shuffle;
      default:
        throw ArgumentError('Invalid value for PlaybackMode: $value');
    }
  }
}
