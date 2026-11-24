import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';

import '../settings_manager.dart';

void setMixWithOthers() async {
  final enabled =
      await SettingsManager().getValue<bool?>(kMixWithOthersKey) == true;

  SetMixWithOthersRequest(enabled: enabled).sendSignalToRust();
}
