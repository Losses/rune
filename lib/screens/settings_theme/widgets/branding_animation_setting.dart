import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class BrandingAnimationSetting extends StatefulWidget {
  const BrandingAnimationSetting({super.key});

  @override
  BrandingAnimationSettingState createState() =>
      BrandingAnimationSettingState();
}

class BrandingAnimationSettingState extends State<BrandingAnimationSetting> {
  bool disableBrandingAnimation = false;

  @override
  void initState() {
    super.initState();
    _loadBrandingAnimationSetting();
  }

  Future<void> _loadBrandingAnimationSetting() async {
    final storedDisableBrandingAnimation =
        await $settingsManager.getValue<bool>(kDisableBrandingAnimationKey);
    setState(() {
      disableBrandingAnimation = storedDisableBrandingAnimation ?? false;
    });
  }

  void _toggleBrandingAnimation(bool value) async {
    setState(() {
      disableBrandingAnimation = !value;
    });
    await $settingsManager.setValue(kDisableBrandingAnimationKey, !value);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxToggle(
      title: s.brandingAnimation,
      subtitle: s.brandingAnimationSubtitle,
      value: !disableBrandingAnimation,
      onChanged: _toggleBrandingAnimation,
    );
  }
}
