import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class PerformanceLevelSetting extends StatefulWidget {
  const PerformanceLevelSetting({super.key});

  @override
  PerformanceLevelSettingState createState() => PerformanceLevelSettingState();
}

class PerformanceLevelSettingState extends State<PerformanceLevelSetting> {
  String performanceLevel = "performance";

  @override
  void initState() {
    super.initState();
    _loadPerformanceLevel();
  }

  Future<void> _loadPerformanceLevel() async {
    final storedPerformanceLevel =
        await $settingsManager.getValue<String>(kAnalysisPerformanceLevelKey);
    setState(() {
      performanceLevel = storedPerformanceLevel ?? "performance";
    });
  }

  Future<void> _updatePerformanceLevel(String newLevel) async {
    setState(() {
      performanceLevel = newLevel;
    });
    await $settingsManager.setValue(kAnalysisPerformanceLevelKey, newLevel);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return SettingsBoxComboBox(
      title: s.performanceLevel,
      subtitle: s.performanceLevelSubtitle,
      value: performanceLevel,
      items: [
        SettingsBoxComboBoxItem(value: "performance", title: s.performance),
        SettingsBoxComboBoxItem(value: "balance", title: s.balance),
        SettingsBoxComboBoxItem(value: "battery", title: s.batterySaving),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updatePerformanceLevel(newValue);
        }
      },
    );
  }
}
