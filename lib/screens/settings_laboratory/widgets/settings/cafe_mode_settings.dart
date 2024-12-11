import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/settings_manager.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/cafe_mode.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

const cafeModeKey = 'cafe_mode';

class CafeModeSettings extends StatefulWidget {
  const CafeModeSettings({super.key});

  @override
  CafeModeSettingsState createState() => CafeModeSettingsState();
}

class CafeModeSettingsState extends State<CafeModeSettings> {
  String? cafeMode = 'false';
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedCafeMode = await _settingsManager.getValue<String>(cafeModeKey);
    if (storedCafeMode != null) {
      setState(() => cafeMode = storedCafeMode);
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = cafeModeConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Café Mode",
      description:
          "Automatically launch into the Cover Art Wall interface. A random track will then be selected in Roaming mode.",
      content: SettingsComboBox<String>(
        value: cafeMode,
        items: items,
        onChanged: (value) {
          if (value == null) return;
          setState(() => cafeMode = value);
          _settingsManager.setValue<String>(cafeModeKey, value);
        },
      ),
    );
  }
}
