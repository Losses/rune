import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class DeduplicateThresholdSetting extends StatefulWidget {
  const DeduplicateThresholdSetting({super.key});

  @override
  DeduplicateThresholdSettingState createState() =>
      DeduplicateThresholdSettingState();
}

class DeduplicateThresholdSettingState
    extends State<DeduplicateThresholdSetting> {
  String similarityLevel = "0.85";

  @override
  void initState() {
    super.initState();
    _loadSimilarityLevel();
  }

  Future<void> _loadSimilarityLevel() async {
    final storedSimilarityLevel =
        await $settingsManager.getValue<String>(kDeduplicateThresholdKey);
    setState(() {
      similarityLevel = storedSimilarityLevel ?? "0.85";
    });
  }

  Future<void> _updateSimilarityLevel(String newLevel) async {
    setState(() {
      similarityLevel = newLevel;
    });
    await $settingsManager.setValue(kDeduplicateThresholdKey, newLevel);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return SettingsBoxComboBox(
      title: s.deduplicateThresholdTitle,
      subtitle: s.deduplicateThresholdSubtitle,
      value: similarityLevel,
      items: [
        SettingsBoxComboBoxItem(
          value: "0.95",
          title: s.nearlyIdentical,
        ),
        SettingsBoxComboBoxItem(
          value: "0.85",
          title: s.highlySimiar,
        ),
        SettingsBoxComboBoxItem(
          value: "0.75",
          title: s.moderatelySimilar,
        ),
        SettingsBoxComboBoxItem(
          value: "0.65",
          title: s.slightlySimilar,
        ),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateSimilarityLevel(newValue);
        }
      },
    );
  }
}
