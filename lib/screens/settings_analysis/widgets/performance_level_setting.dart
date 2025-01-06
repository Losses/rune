import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/settings_manager.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';

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
        await SettingsManager().getValue<String>(kAnalysisPerformanceLevelKey);
    setState(() {
      performanceLevel = storedPerformanceLevel ?? "performance";
    });
  }

  Future<void> _updatePerformanceLevel(String newLevel) async {
    setState(() {
      performanceLevel = newLevel;
    });
    await SettingsManager().setValue(kAnalysisPerformanceLevelKey, newLevel);
  }

  @override
  Widget build(BuildContext context) {
    return SettingsBoxComboBox(
      title: S.of(context).performanceLevel,
      subtitle: S.of(context).performanceLevelSubtitle,
      value: performanceLevel,
      items: [
        SettingsBoxComboBoxItem(
            value: "performance", title: S.of(context).performance),
        SettingsBoxComboBoxItem(value: "balance", title: S.of(context).balance),
        SettingsBoxComboBoxItem(
            value: "battery", title: S.of(context).batterySaving),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updatePerformanceLevel(newValue);
        }
      },
    );
  }
}
