import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

void setAdaptiveSwitchingEnabled() async {
  final enabled =
      await SettingsManager().getValue<bool?>(kAdaptiveSwitchingKey) == true;

  SetAdaptiveSwitchingEnabledRequest(enabled: enabled).sendSignalToRust();
}
