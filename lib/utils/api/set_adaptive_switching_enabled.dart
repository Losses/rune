import '../../messages/all.dart';
import '../../screens/settings_playback/settings_playback.dart';
import '../settings_manager.dart';

void setAdaptiveSwitchingEnabled() async {
  final enabled =
      await SettingsManager().getValue<bool?>(adaptiveSwitchingKey) == true;

  SetAdaptiveSwitchingEnabledRequest(enabled: enabled).sendSignalToRust();
}
