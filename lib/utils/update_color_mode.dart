import 'package:fluent_ui/fluent_ui.dart';

import '../config/theme.dart';

updateColorMode(String? colorMode) {
  if (colorMode == 'dark') {
    appTheme.mode = ThemeMode.dark;
  } else if (colorMode == 'light') {
    appTheme.mode = ThemeMode.light;
  } else {
    appTheme.mode = ThemeMode.system;
  }
}
