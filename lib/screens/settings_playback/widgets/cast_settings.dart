import 'package:fluent_ui/fluent_ui.dart';

import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';
import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';

class CastTrackModeSetting extends StatefulWidget {
  const CastTrackModeSetting({super.key});

  @override
  State<CastTrackModeSetting> createState() => _CastTrackModeSettingState();
}

class _CastTrackModeSettingState extends State<CastTrackModeSetting> {
  String _value = 'replace';

  @override
  void initState() {
    super.initState();
    $settingsManager.getValue<String>(kCastTrackModeKey).then((value) {
      if (mounted) setState(() => _value = value ?? 'replace');
    });
  }

  Future<void> _set(String value) async {
    setState(() => _value = value);
    await $settingsManager.setValue(kCastTrackModeKey, value);
  }

  @override
  Widget build(BuildContext context) {
    final strings = S.of(context);
    return SettingsBoxComboBox(
      title: strings.castTrackMode,
      subtitle: strings.castTrackModeSubtitle,
      value: _value,
      items: [
        SettingsBoxComboBoxItem(
          value: 'replace',
          title: strings.castTrackReplace,
        ),
        SettingsBoxComboBoxItem(
          value: 'playNext',
          title: strings.castTrackPlayNext,
        ),
      ],
      onChanged: (value) {
        if (value != null) _set(value);
      },
    );
  }
}

class CastControlRendererSetting extends StatefulWidget {
  const CastControlRendererSetting({super.key});

  @override
  State<CastControlRendererSetting> createState() =>
      _CastControlRendererSettingState();
}

class _CastControlRendererSettingState
    extends State<CastControlRendererSetting> {
  String _value = 'false';

  @override
  void initState() {
    super.initState();
    $settingsManager.getValue<bool>(kCastControlRendererKey).then((value) {
      if (mounted) setState(() => _value = '${value ?? false}');
    });
  }

  Future<void> _set(String value) async {
    setState(() => _value = value);
    await $settingsManager.setValue(kCastControlRendererKey, value == 'true');
  }

  @override
  Widget build(BuildContext context) {
    final strings = S.of(context);
    return SettingsBoxComboBox(
      title: strings.castControlRenderer,
      subtitle: strings.castControlRendererSubtitle,
      value: _value,
      items: [
        SettingsBoxComboBoxItem(
          value: 'false',
          title: strings.castKeepControlling,
        ),
        SettingsBoxComboBoxItem(
          value: 'true',
          title: strings.castControlCastDevice,
        ),
      ],
      onChanged: (value) {
        if (value != null) _set(value);
      },
    );
  }
}
