import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

import '../constants/window_sizes.dart';

class WindowSizeSetting extends StatefulWidget {
  const WindowSizeSetting({super.key});

  @override
  WindowSizeSettingState createState() => WindowSizeSettingState();
}

class WindowSizeSettingState extends State<WindowSizeSetting> {
  String windowSize = "normal";

  @override
  void initState() {
    super.initState();
    _loadWindowSizeSetting();
  }

  Future<void> _loadWindowSizeSetting() async {
    final storedWindowSize =
        await $settingsManager.getValue<String>(kWindowSizeKey);
    setState(() {
      windowSize = storedWindowSize ?? "normal";
    });
  }

  void _updateWindowSize(String newSize) async {
    setState(() {
      windowSize = newSize;
    });
    await $settingsManager.setValue(kWindowSizeKey, newSize);

    final firstView = WidgetsBinding.instance.platformDispatcher.views.first;
    final size = Platform.isWindows || Platform.isMacOS
        ? windowSizes[newSize]!
        : windowSizes[newSize]! / firstView.devicePixelRatio;
    appWindow.size = size;
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.windowSize,
      subtitle: s.windowSizeSubtitle,
      value: windowSize,
      items: [
        SettingsBoxComboBoxItem(value: "normal", title: s.normalWindowSize),
        SettingsBoxComboBoxItem(value: "slim", title: s.slimWindowSize),
        SettingsBoxComboBoxItem(value: "stocky", title: s.stockyWindowSize),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateWindowSize(newValue);
        }
      },
    );
  }
}
