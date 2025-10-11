import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_toggle.dart';
import '../../../providers/linux_custom_window_controls.dart';

class LinuxCustomWindowControlsSetting extends StatefulWidget {
  const LinuxCustomWindowControlsSetting({super.key});

  @override
  State<LinuxCustomWindowControlsSetting> createState() =>
      _LinuxCustomWindowControlsSettingState();
}

class _LinuxCustomWindowControlsSettingState
    extends State<LinuxCustomWindowControlsSetting> {
  @override
  void initState() {
    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    // Only show this setting on Linux
    if (!Platform.isLinux) {
      return const SizedBox.shrink();
    }

    return Consumer<LinuxCustomWindowControlsProvider>(
      builder: (context, provider, child) {
        return SettingsBoxToggle(
          title: s.linuxCustomWindowControls,
          subtitle: s.linuxCustomWindowControlsSubtitle,
          value: provider.enabled,
          onChanged: (newValue) {
            provider.setEnabled(newValue);
          },
        );
      },
    );
  }
}