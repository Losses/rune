import '../../messages/all.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

void playMode(int mode) async {
  await SettingsManager().setValue(kPlaybackModeKey, mode);

  SetPlaybackModeRequest(mode: mode).sendSignalToRust();
}
