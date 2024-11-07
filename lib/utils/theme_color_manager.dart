import 'package:flutter/material.dart';

import '../config/theme.dart';
import '../screens/settings_theme/settings_theme.dart';

import 'api/get_primary_color.dart';
import 'settings_manager.dart';

class ThemeColorManager {
  static final ThemeColorManager _instance = ThemeColorManager._internal();
  factory ThemeColorManager() => _instance;
  ThemeColorManager._internal();

  bool _isDynamicColorEnabled = false;
  Color? _userSelectedColor;
  int? _currentCoverArtId;

  Future<void> initialize() async {
    final settingsManager = SettingsManager();

    // 1. Read dynamic color settings
    final bool? enableDynamicColors =
        await settingsManager.getValue<bool>(enableDynamicColorsKey);
    _isDynamicColorEnabled = enableDynamicColors ?? false;

    // 2. Read the user's selected theme color
    final int? themeColor = await settingsManager.getValue<int>(themeColorKey);
    if (themeColor != null) {
      _userSelectedColor = Color(themeColor);
    }
  }

  // Update dynamic color settings
  Future<void> updateDynamicColorSetting(bool enabled) async {
    _isDynamicColorEnabled = enabled;

    if (enabled) {
      // If dynamic colors are enabled and a song is currently playing, apply the cover color immediately
      if (_currentCoverArtId != null) {
        await handleCoverArtColorChange(_currentCoverArtId!);
      }
    } else {
      // If dynamic colors are disabled, decide which color to use based on user settings
      // If the user chose to follow the system, pass in null
      appTheme.updateThemeColor(_userSelectedColor);
    }
  }

  // Update the user's selected theme color
  void updateUserSelectedColor(Color? color) {
    _userSelectedColor = color;
    // If dynamic colors are not enabled, update the theme based on user selection
    if (!_isDynamicColorEnabled) {
      appTheme.updateThemeColor(color);
    }
  }

  // Handle cover art color change
  Future<void> handleCoverArtColorChange(int coverArtId) async {
    _currentCoverArtId = coverArtId;

    if (!_isDynamicColorEnabled) return;

    final primaryColor = await getPrimaryColor(coverArtId);
    if (primaryColor != null) {
      appTheme.updateThemeColor(Color(primaryColor));
    } else {
      appTheme.updateThemeColor(_userSelectedColor);
    }
  }

  // Get the current color that should be used
  Future<void> refreshCurrentColor() async {
    if (_isDynamicColorEnabled && _currentCoverArtId != null) {
      await handleCoverArtColorChange(_currentCoverArtId!);
    } else {
      appTheme.updateThemeColor(_userSelectedColor);
    }
  }
}
