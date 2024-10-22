import '../messages/all.dart';

import 'settings_manager.dart';

const String nonReplaceOperateModeKey = 'playlist_operate_mode';

final SettingsManager settingsManager = SettingsManager();

Future<PlaylistOperateMode> getNonReplaceOperateMode() async {
  String? storedVolume =
      await settingsManager.getValue<String>(nonReplaceOperateModeKey);

  return storedVolume == 'play_next'
      ? PlaylistOperateMode.PlayNext
      : PlaylistOperateMode.AppendToEnd;
}
