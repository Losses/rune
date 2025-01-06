import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/settings_manager.dart';
import '../../../../constants/configurations.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/cover_wall_item_count.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class CoverWallRichnessSettings extends StatefulWidget {
  const CoverWallRichnessSettings({super.key});

  @override
  CoverWallRichnessSettingsState createState() =>
      CoverWallRichnessSettingsState();
}

class CoverWallRichnessSettingsState extends State<CoverWallRichnessSettings> {
  String? randomCoverWallCount = '40';
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedCount =
        await _settingsManager.getValue<String>(kRandomCoverWallCountKey);
    if (storedCount != null) {
      setState(() => randomCoverWallCount = storedCount);
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = randomCoverWallCountConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Cover Wall Richness",
      description:
          "Customize the maximum number of covers displayed on the cover wall. Please note that having too many may lead to insufficient operating system memory.",
      content: SettingsComboBox<String>(
        value: randomCoverWallCount,
        items: items,
        onChanged: (value) {
          if (value == null) return;
          setState(() => randomCoverWallCount = value);
          _settingsManager.setValue<String>(kRandomCoverWallCountKey, value);
        },
      ),
    );
  }
}
