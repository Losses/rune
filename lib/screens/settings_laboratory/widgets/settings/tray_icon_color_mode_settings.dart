import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/tray_manager.dart';
import '../../../../utils/settings_manager.dart';
import '../../../../constants/configurations.dart';

import '../../utils/settings_combo_box_item.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class TrayIconColorModeSettings extends StatefulWidget {
  const TrayIconColorModeSettings({super.key});

  @override
  TrayIconColorModeSettingsState createState() =>
      TrayIconColorModeSettingsState();
}

class TrayIconColorModeSettingsState extends State<TrayIconColorModeSettings> {
  String? trayIconColorMode = 'auto';
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedMode =
        await _settingsManager.getValue<String>(kTrayIconColorModeKey);
    if (storedMode != null) {
      setState(() => trayIconColorMode = storedMode);
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = [
      SettingsComboBoxItem(
        value: 'auto',
        text: 'Automatic',
      ),
      SettingsComboBoxItem(
        value: 'light',
        text: 'Light',
      ),
      SettingsComboBoxItem(
        value: 'dark',
        text: 'Dark',
      ),
    ];

    return SettingsCard(
      title: "Tray Icon Color Mode",
      description:
          "Set the tray icon color mode preference. Automatic mode will adapt to your system theme.",
      content: SettingsComboBox<String>(
        value: trayIconColorMode,
        items: items,
        onChanged: (value) async {
          if (value == null) return;
          setState(() => trayIconColorMode = value);
          await _settingsManager.setValue<String>(kTrayIconColorModeKey, value);

          // Update the tray icon immediately
          $tray.updateTrayIcon();
        },
      ),
    );
  }
}
