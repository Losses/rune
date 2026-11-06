import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/set_mix_with_others.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class MixWithOthersSetting extends StatefulWidget {
  const MixWithOthersSetting({super.key});

  @override
  MixWithOthersSettingState createState() => MixWithOthersSettingState();
}

class MixWithOthersSettingState extends State<MixWithOthersSetting> {
  bool mixWithOthers = false;

  @override
  void initState() {
    super.initState();
    _loadMixWithOthers();
  }

  Future<void> _loadMixWithOthers() async {
    final stored = await $settingsManager.getValue<bool>(kMixWithOthersKey);
    if (!mounted) return;
    setState(() {
      mixWithOthers = stored ?? false;
    });
  }

  Future<void> _updateMixWithOthers(bool newSetting) async {
    setState(() {
      mixWithOthers = newSetting;
    });
    await $settingsManager.setValue(kMixWithOthersKey, newSetting);
    setMixWithOthers();
  }

  @override
  Widget build(BuildContext context) {
    // Only iOS has an audio session to mix in.
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);

    return SettingsBoxToggle(
      title: s.mixWithOthers,
      subtitle: s.mixWithOthersSubtitle,
      value: mixWithOthers,
      onChanged: _updateMixWithOthers,
    );
  }
}
