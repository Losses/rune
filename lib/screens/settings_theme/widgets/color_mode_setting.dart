import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/update_color_mode.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class ColorModeSetting extends StatefulWidget {
  const ColorModeSetting({super.key});

  @override
  ColorModeSettingState createState() => ColorModeSettingState();
}

class ColorModeSettingState extends State<ColorModeSetting> {
  String colorMode = "system";

  @override
  void initState() {
    super.initState();
    _loadColorMode();
  }

  Future<void> _loadColorMode() async {
    final storedColorMode =
        await $settingsManager.getValue<String>(kColorModeKey);
    setState(() {
      colorMode = storedColorMode ?? "system";
    });
  }

  Future<void> _updateColorMode(String newMode) async {
    setState(() {
      colorMode = newMode;
      updateColorMode(colorMode);
    });
    await $settingsManager.setValue(kColorModeKey, newMode);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.colorMode,
      subtitle: s.colorModeSubtitle,
      value: colorMode,
      items: [
        SettingsBoxComboBoxItem(value: "system", title: s.systemColorMode),
        SettingsBoxComboBoxItem(value: "dark", title: s.dark),
        SettingsBoxComboBoxItem(value: "light", title: s.light),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateColorMode(newValue);
        }
      },
    );
  }
}
