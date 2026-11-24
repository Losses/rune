import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/settings_manager.dart';
import '../../../../constants/configurations.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/mild_spectrum.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class MildSpectrumSettings extends StatefulWidget {
  const MildSpectrumSettings({super.key});

  @override
  MildSpectrumSettingsState createState() => MildSpectrumSettingsState();
}

class MildSpectrumSettingsState extends State<MildSpectrumSettings> {
  String mildSpectrum = 'false';
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedMildSpectrum =
        await _settingsManager.getValue<String>(kMildSpectrumKey);
    if (storedMildSpectrum != null) {
      setState(() => mildSpectrum = storedMildSpectrum);
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = mildSpectrumConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Mild Spectrum",
      description:
          "Enjoy a softer visual experience with reduced motion and gentle animations. Ideal for a nostalgic feel.",
      content: SettingsComboBox<String>(
        value: mildSpectrum == 'true' ? 'true' : 'false',
        items: items,
        onChanged: (value) {
          if (value == null) return;
          setState(() => mildSpectrum = value);
          _settingsManager.setValue<String>(kMildSpectrumKey, value);
        },
      ),
    );
  }
}
