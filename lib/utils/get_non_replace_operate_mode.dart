import '../messages/all.dart';
import '../constants/configurations.dart';

import 'settings_manager.dart';

final SettingsManager settingsManager = SettingsManager();

Future<PlaylistOperateMode> getNonReplaceOperateMode() async {
  String? storedVolume =
      await settingsManager.getValue<String>(kNonReplaceOperateModeKey);

  return storedVolume == 'PlayNext'
      ? PlaylistOperateMode.PlayNext
      : PlaylistOperateMode.AppendToEnd;
}
