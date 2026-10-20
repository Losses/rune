import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../theme.dart';

/// Whether the app looks light, from its own theme mode rather than the
/// nearest [FluentTheme], which the cover wall may have darkened. The
/// visualizer's default preset has a light and a dark version.
bool isAppLight(BuildContext context) {
  return switch (context.watch<AppTheme>().mode) {
    ThemeMode.light => true,
    ThemeMode.dark => false,
    ThemeMode.system =>
      MediaQuery.platformBrightnessOf(context) == Brightness.light,
  };
}

/// The background of the light default preset (`presets/Rune Light.milk`).
const visualizerLightPaper = Color(0xFFF9F4F6);
