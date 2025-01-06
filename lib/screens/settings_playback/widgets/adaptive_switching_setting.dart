import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/set_adaptive_switching_enabled.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class AdaptiveSwitchingSetting extends StatefulWidget {
  const AdaptiveSwitchingSetting({super.key});

  @override
  AdaptiveSwitchingSettingState createState() =>
      AdaptiveSwitchingSettingState();
}

class AdaptiveSwitchingSettingState extends State<AdaptiveSwitchingSetting> {
  bool adaptiveSwitching = false;

  @override
  void initState() {
    super.initState();
    _loadAdaptiveSwitching();
  }

  Future<void> _loadAdaptiveSwitching() async {
    final storedAdaptiveSwitching =
        await $settingsManager.getValue<bool>(kAdaptiveSwitchingKey);
    setState(() {
      adaptiveSwitching = storedAdaptiveSwitching ?? false;
    });
  }

  Future<void> _updateAdaptiveSwitching(bool newSetting) async {
    setState(() {
      adaptiveSwitching = newSetting;
    });
    await $settingsManager.setValue(kAdaptiveSwitchingKey, newSetting);
    setAdaptiveSwitchingEnabled();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxToggle(
      title: s.adaptiveSwitching,
      subtitle: s.adaptiveSwitchingSubtitle,
      value: adaptiveSwitching,
      onChanged: _updateAdaptiveSwitching,
    );
  }
}
