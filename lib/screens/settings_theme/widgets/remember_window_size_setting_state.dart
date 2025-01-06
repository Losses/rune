import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class RememberWindowSizeSetting extends StatefulWidget {
  const RememberWindowSizeSetting({super.key});

  @override
  RememberWindowSizeSettingState createState() =>
      RememberWindowSizeSettingState();
}

class RememberWindowSizeSettingState extends State<RememberWindowSizeSetting> {
  bool rememberWindowSize = false;

  @override
  void initState() {
    super.initState();
    _loadRememberWindowSizeSetting();
  }

  Future<void> _loadRememberWindowSizeSetting() async {
    final storedRememberWindowSize =
        await $settingsManager.getValue<bool>(kRememberWindowSizeKey);
    setState(() {
      rememberWindowSize = storedRememberWindowSize ?? false;
    });
  }

  void _toggleRememberWindowSize(bool value) async {
    setState(() {
      rememberWindowSize = value;
    });
    await $settingsManager.setValue(kRememberWindowSizeKey, value);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxToggle(
      title: s.rememberWindowSize,
      subtitle: s.rememberWindowSizeSubtitle,
      value: rememberWindowSize,
      onChanged: _toggleRememberWindowSize,
    );
  }
}
