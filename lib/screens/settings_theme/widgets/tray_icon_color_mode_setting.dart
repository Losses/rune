import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/tray_manager.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class TrayIconColorModeSetting extends StatefulWidget {
  const TrayIconColorModeSetting({super.key});

  @override
  TrayIconColorModeSettingState createState() =>
      TrayIconColorModeSettingState();
}

class TrayIconColorModeSettingState extends State<TrayIconColorModeSetting> {
  String trayIconColorMode = "auto";

  @override
  void initState() {
    super.initState();
    _loadTrayIconColorMode();
  }

  Future<void> _loadTrayIconColorMode() async {
    final storedMode =
        await $settingsManager.getValue<String>(kTrayIconColorModeKey);
    setState(() {
      trayIconColorMode = storedMode ?? "auto";
    });
  }

  Future<void> _updateTrayIconColorMode(String newMode) async {
    setState(() {
      trayIconColorMode = newMode;
    });
    await $settingsManager.setValue(kTrayIconColorModeKey, newMode);

    // Update the tray icon immediately
    $tray.updateTrayIcon();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.trayIconColorMode,
      subtitle: s.trayIconColorModeSubtitle,
      value: trayIconColorMode,
      items: [
        SettingsBoxComboBoxItem(
            value: "auto", title: s.automaticTrayIconMode),
        SettingsBoxComboBoxItem(value: "light", title: s.light),
        SettingsBoxComboBoxItem(value: "dark", title: s.dark),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateTrayIconColorMode(newValue);
        }
      },
    );
  }
}
