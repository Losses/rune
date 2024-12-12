import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../../utils/settings_manager.dart';
import '../../../../providers/responsive_providers.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/force_zune.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class ForceZuneSettings extends StatefulWidget {
  const ForceZuneSettings({super.key});

  @override
  ForceZuneSettingsState createState() => ForceZuneSettingsState();
}

class ForceZuneSettingsState extends State<ForceZuneSettings> {
  bool? zuneMode = false;
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedZuneMode =
        await _settingsManager.getValue<String>(forceLayoutModeKey);
    if (storedZuneMode != null) {
      setState(() => zuneMode = storedZuneMode == 'zune');
    }
  }

  @override
  Widget build(BuildContext context) {
    final r = Provider.of<ResponsiveProvider>(context, listen: false);

    final items = forceZuneConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Force Zune Mode",
      description:
          "Force the responsive grid to resolved as Zune mode, offering you a mobile experience.",
      content: SettingsComboBox<String>(
        value: zuneMode == true ? 'true' : 'false',
        items: items,
        onChanged: (value) {
          if (value == null) return;
          setState(() => zuneMode = value == 'true');
          if (value == 'true') {
            _settingsManager.setValue<String>(forceLayoutModeKey, 'zune');
          } else {
            _settingsManager.removeValue(forceLayoutModeKey);
          }

          r.updateForceLayoutConfig();
        },
      ),
    );
  }
}
