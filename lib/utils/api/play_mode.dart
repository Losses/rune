import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

void playMode(
  PlaybackMode mode, {
  Future<void> Function(PlaybackMode mode)? persist,
  void Function(PlaybackMode mode)? send,
}) async {
  (send ?? _sendPlaybackMode)(mode);
  await (persist ?? _persistPlaybackMode)(mode);
}

void _sendPlaybackMode(PlaybackMode mode) {
  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}

Future<void> _persistPlaybackMode(PlaybackMode mode) {
  return SettingsManager().setValue(kPlaybackModeKey, mode.index);
}
