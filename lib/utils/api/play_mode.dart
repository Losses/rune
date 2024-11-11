import '../../messages/all.dart';
import '../settings_manager.dart';

const playbackModeKey = 'playback_mode';

void playMode(int mode) async {
  await SettingsManager().setValue(playbackModeKey, mode);

  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}
