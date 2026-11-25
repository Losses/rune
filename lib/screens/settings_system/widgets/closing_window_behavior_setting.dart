import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class ClosingWindowBehaviorSetting extends StatefulWidget {
  const ClosingWindowBehaviorSetting({super.key});

  @override
  ClosingWindowBehaviorSettingState createState() =>
      ClosingWindowBehaviorSettingState();
}

class ClosingWindowBehaviorSettingState
    extends State<ClosingWindowBehaviorSetting> {
  String closingWindowBehavior = "tray";

  @override
  void initState() {
    super.initState();
    _loadClosingWindowBehavior();
  }

  Future<void> _loadClosingWindowBehavior() async {
    final storedClosingWindowBehavior =
        await $settingsManager.getValue<String>(kClosingWindowBehaviorKey);
    setState(() {
      closingWindowBehavior = storedClosingWindowBehavior ?? "tray";
    });
  }

  Future<void> _updateClosingWindowBehavior(String newLevel) async {
    setState(() {
      closingWindowBehavior = newLevel;
    });
    await $settingsManager.setValue(kClosingWindowBehaviorKey, newLevel);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return SettingsBoxComboBox(
      title: s.closingWindowBehaviorTitle,
      subtitle: s.closingWindowBehaviorSubtitle,
      value: closingWindowBehavior,
      items: [
        SettingsBoxComboBoxItem(value: "tray", title: s.minimizeToTray),
        SettingsBoxComboBoxItem(value: "exit", title: s.exitProgram),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateClosingWindowBehavior(newValue);
        }
      },
    );
  }
}
