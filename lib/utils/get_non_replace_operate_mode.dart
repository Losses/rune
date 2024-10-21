import '../messages/all.dart';

import 'settings_manager.dart';

const String _volumeSettingsKey = 'playlist_operate_mode';

final SettingsManager settingsManager = SettingsManager();

Future<PlaylistOperateMode> getNonReplaceOperateMode() async {
  String? storedVolume =
      await settingsManager.getValue<String>(_volumeSettingsKey);

  return storedVolume == 'play_next'
      ? PlaylistOperateMode.PlayNext
      : PlaylistOperateMode.AppendToEnd;
}
