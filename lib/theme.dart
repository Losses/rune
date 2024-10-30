import 'dart:io' show Platform;

import 'package:flutter/foundation.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:system_theme/system_theme.dart';
import 'package:flutter_acrylic/flutter_acrylic.dart';
import 'package:material_color_utilities/material_color_utilities.dart';


enum NavigationIndicators { sticky, end }

class AppTheme extends ChangeNotifier {
  AccentColor? _color;
  AccentColor get color => _color ?? systemAccentColor;
  set color(AccentColor color) {
    _color = color;
    notifyListeners();
  }

  _getAccentColor(Color? color) {
    if (color == null) {
      _color = systemAccentColor;

      return;
    }

    final colorScheme = Hct.fromInt(color.value);
    final double h = colorScheme.hue;
    final double c = colorScheme.chroma;
    final double t = colorScheme.tone.clamp(40, 70);

    return AccentColor.swatch({
      'darkest': Color(Hct.from(h, c, t - 19).toInt()),
      'darker': Color(Hct.from(h, c, t - 15).toInt()),
      'dark': Color(Hct.from(h, c, t - 8).toInt()),
      'normal': Color(Hct.from(h, c, t).toInt()),
      'light': Color(Hct.from(h, c, t + 6).toInt()),
      'lighter': Color(Hct.from(h, c, t + 13).toInt()),
      'lightest': Color(Hct.from(h, c, t + 17).toInt()),
    });
  }

  updateThemeColor(Color? color) {
    _color = _getAccentColor(color);
    notifyListeners();
  }

  ThemeMode _mode = ThemeMode.system;
  ThemeMode get mode => _mode;
  set mode(ThemeMode mode) {
    _mode = mode;
    notifyListeners();
  }

  final PaneDisplayMode displayMode = PaneDisplayMode.top;

  WindowEffect windowEffect = (Platform.isLinux || Platform.isAndroid)
      ? WindowEffect.solid
      : WindowEffect.mica;

  TextDirection _textDirection = TextDirection.ltr;
  TextDirection get textDirection => _textDirection;
  set textDirection(TextDirection direction) {
    _textDirection = direction;
    notifyListeners();
  }

  Locale? _locale;
  Locale? get locale => _locale;
  set locale(Locale? locale) {
    _locale = locale;
    notifyListeners();
  }

  void setEffect(FluentThemeData theme) {
    Window.setEffect(
      effect: windowEffect,
      color: windowEffect != WindowEffect.mica
          ? theme.brightness == Brightness.light
              ? const Color(0xFFF6F6F6)
              : const Color(0xFF1F1F1F)
          : theme.micaBackgroundColor.withOpacity(0.05),
      dark: theme.brightness.isDark,
    );
  }
}

AccentColor get systemAccentColor {
  if ((defaultTargetPlatform == TargetPlatform.windows ||
          defaultTargetPlatform == TargetPlatform.android) &&
      !kIsWeb) {
    return AccentColor.swatch({
      'darkest': SystemTheme.accentColor.darkest,
      'darker': SystemTheme.accentColor.darker,
      'dark': SystemTheme.accentColor.dark,
      'normal': SystemTheme.accentColor.accent,
      'light': SystemTheme.accentColor.light,
      'lighter': SystemTheme.accentColor.lighter,
      'lightest': SystemTheme.accentColor.lightest,
    });
  }
  return Colors.blue;
}
