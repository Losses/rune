import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/theme_color_manager.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class DynamicColorsSetting extends StatefulWidget {
  const DynamicColorsSetting({super.key});

  @override
  DynamicColorsSettingState createState() => DynamicColorsSettingState();
}

class DynamicColorsSettingState extends State<DynamicColorsSetting> {
  bool enableDynamicColors = false;

  @override
  void initState() {
    super.initState();
    _loadDynamicColorsSetting();
  }

  Future<void> _loadDynamicColorsSetting() async {
    final storedEnableDynamicColors =
        await $settingsManager.getValue<bool>(kEnableDynamicColorsKey);
    setState(() {
      enableDynamicColors = storedEnableDynamicColors ?? false;
    });
  }

  void _handleDynamicColorToggle(bool value) async {
    setState(() {
      enableDynamicColors = value;
    });
    await $settingsManager.setValue(kEnableDynamicColorsKey, value);
    ThemeColorManager().updateDynamicColorSetting(value);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxToggle(
      title: s.dynamicColors,
      subtitle: s.dynamicColorsSubtitle,
      value: enableDynamicColors,
      onChanged: _handleDynamicColorToggle,
    );
  }
}
