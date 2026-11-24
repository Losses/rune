import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class QueueModeSetting extends StatefulWidget {
  const QueueModeSetting({super.key});

  @override
  QueueModeSettingState createState() => QueueModeSettingState();
}

class QueueModeSettingState extends State<QueueModeSetting> {
  String queueMode = "AddToEnd";

  @override
  void initState() {
    super.initState();
    _loadQueueMode();
  }

  Future<void> _loadQueueMode() async {
    final storedQueueSetting =
        await $settingsManager.getValue<String>(kNonReplaceOperateModeKey);
    setState(() {
      queueMode = storedQueueSetting ?? "AddToEnd";
    });
  }

  Future<void> _updateQueueSetting(String newSetting) async {
    setState(() {
      queueMode = newSetting;
    });
    await $settingsManager.setValue(kNonReplaceOperateModeKey, newSetting);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.addToQueue,
      subtitle: s.addToQueueSubtitle,
      value: queueMode,
      items: [
        SettingsBoxComboBoxItem(value: "PlayNext", title: s.playNext),
        SettingsBoxComboBoxItem(value: "AddToEnd", title: s.addToEnd),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateQueueSetting(newValue);
        }
      },
    );
  }
}
