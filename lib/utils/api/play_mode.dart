import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

void playMode(
  int mode, {
  Future<void> Function(int mode)? persist,
  void Function(int mode)? send,
}) async {
  (send ?? _sendPlaybackMode)(mode);
  await (persist ?? _persistPlaybackMode)(mode);
}

void _sendPlaybackMode(int mode) {
  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}

Future<void> _persistPlaybackMode(int mode) {
  return SettingsManager().setValue(kPlaybackModeKey, mode);
}
