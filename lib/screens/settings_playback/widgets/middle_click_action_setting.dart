import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class MiddleClickActionSetting extends StatefulWidget {
  const MiddleClickActionSetting({super.key});

  @override
  MiddleClickActionSettingState createState() =>
      MiddleClickActionSettingState();
}

class MiddleClickActionSettingState extends State<MiddleClickActionSetting> {
  String middleClickAction = "StartPlaying";

  @override
  void initState() {
    super.initState();
    _loadMiddleClickAction();
  }

  Future<void> _loadMiddleClickAction() async {
    final storedMiddleClickAction =
        await $settingsManager.getValue<String>(kMiddleClickActionKey);
    setState(() {
      middleClickAction = storedMiddleClickAction ?? "StartPlaying";
    });
  }

  Future<void> _updateMiddleClickAction(String newAction) async {
    setState(() {
      middleClickAction = newAction;
    });
    await $settingsManager.setValue(kMiddleClickActionKey, newAction);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.middleClickAction,
      subtitle: s.middleClickActionSubtitle,
      value: middleClickAction,
      items: [
        SettingsBoxComboBoxItem(value: "StartPlaying", title: s.startPlaying),
        SettingsBoxComboBoxItem(value: "AddToQueue", title: s.addToQueue),
        SettingsBoxComboBoxItem(value: "StartRoaming", title: s.startRoaming),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateMiddleClickAction(newValue);
        }
      },
    );
  }
}
