import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/color_brightness.dart';

List<Color> getTileColors(FluentThemeData theme) {
  final List<Color> colors = [
    theme.accentColor.darker,
    theme.accentColor.darken(0.1),
    theme.accentColor.darken(0.15),
    theme.accentColor.darken(0.2),
    theme.accentColor.darken(0.25),
  ];

  return colors;
}
