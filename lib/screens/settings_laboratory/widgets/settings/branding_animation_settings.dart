import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/settings_manager.dart';
import '../../../../constants/configurations.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/branding_sfx.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class BrandingAnimationSettings extends StatefulWidget {
  const BrandingAnimationSettings({super.key});

  @override
  BrandingAnimationSettingsState createState() =>
      BrandingAnimationSettingsState();
}

class BrandingAnimationSettingsState extends State<BrandingAnimationSettings> {
  String? brandingSfx = 'fantasy';
  final SettingsManager _settingsManager = SettingsManager();

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    final storedSfx = await _settingsManager.getValue<String>(kBandingSfxKey);
    if (storedSfx != null) {
      setState(() => brandingSfx = storedSfx);
    }
  }

  @override
  Widget build(BuildContext context) {
    final items = bandingSfxConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Branding Animation SFX",
      description:
          "Customize the sound effects for your branding animation. Choose from two unique styles designed by our collaborator, musician Sh4-RA.",
      content: SettingsComboBox<String>(
        value: brandingSfx,
        items: items,
        onChanged: (value) {
          if (value == null) return;
          setState(() => brandingSfx = value);
          _settingsManager.setValue<String>(kBandingSfxKey, value);
        },
      ),
    );
  }
}
