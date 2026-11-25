import '../bindings/bindings.dart';
import 'api/request_failure.dart';
import 'dart:ui';

import '../config/theme.dart';
import '../constants/configurations.dart';

import 'api/get_primary_color.dart';

import 'playing_item.dart';
import 'settings_manager.dart';

class ThemeColorManager {
  static final ThemeColorManager _instance = ThemeColorManager._internal();
  factory ThemeColorManager() => _instance;
  ThemeColorManager._internal();

  bool _isDynamicColorEnabled = false;
  Color? _userSelectedColor;
  PlayingItem? _currentPlayingItem;

  // What the library of the current item declared about its color when the
  // item started, kept to apply the color again for the same item.
  RequestCapability _currentColorCapability =
      const RequestCapabilityAvailable();

  // Counts the calls that pick the color for an item. A color that arrives
  // after a later call, as for the next item, is not applied.
  int _colorRequest = 0;

  Future<void> initialize() async {
    final settingsManager = SettingsManager();

    // 1. Read dynamic color settings
    final bool? enableDynamicColors = await settingsManager.getValue<bool>(
      kEnableDynamicColorsKey,
    );
    _isDynamicColorEnabled = enableDynamicColors ?? false;

    // 2. Read the user's selected theme color
    final int? themeColor = await settingsManager.getValue<int>(kThemeColorKey);
    if (themeColor != null) {
      _userSelectedColor = Color(themeColor);
    }
  }

  // Update dynamic color settings
  Future<void> updateDynamicColorSetting(bool enabled) async {
    _isDynamicColorEnabled = enabled;

    if (enabled) {
      // If dynamic colors are enabled and a song is currently playing, apply the cover color immediately
      if (_currentPlayingItem != null) {
        await handleCoverArtColorChange(
          _currentPlayingItem!,
          _currentColorCapability,
        );
      } else {
        appTheme.updateThemeColor(_userSelectedColor);
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
    } else {
      if (_currentPlayingItem != null) {
        handleCoverArtColorChange(
          _currentPlayingItem!,
          _currentColorCapability,
        );
      } else {
        appTheme.updateThemeColor(color);
      }
    }
  }

  // Handle cover art color change
  // [colorCapability] is what the library of [item] declares about its color.
  Future<void> handleCoverArtColorChange(
    PlayingItem item,
    RequestCapability colorCapability,
  ) async {
    final request = ++_colorRequest;
    _currentPlayingItem = item;
    _currentColorCapability = colorCapability;

    if (!_isDynamicColorEnabled) return;

    int? primaryColor;
    if (asksForColor(colorCapability)) {
      try {
        primaryColor = await getPrimaryColor(item);
      } on RequestFailure catch (error) {
        if (!noColorFailure(error)) rethrow;
      }
      if (request != _colorRequest || !_isDynamicColorEnabled) return;
    }
    if (primaryColor != null) {
      appTheme.updateThemeColor(Color(primaryColor));
    } else {
      appTheme.updateThemeColor(_userSelectedColor);
    }
  }

  // Get the current color that should be used
  Future<void> refreshCurrentColor() async {
    if (_isDynamicColorEnabled && _currentPlayingItem != null) {
      await handleCoverArtColorChange(
        _currentPlayingItem!,
        _currentColorCapability,
      );
    } else {
      appTheme.updateThemeColor(_userSelectedColor);
    }
  }
}

/// Whether the color of an item's cover is asked for, given what its library
/// declares: an item whose library gives no color for it, as a file without a
/// local library to read it through, is not asked, as asking would only fail.
bool asksForColor(RequestCapability capability) =>
    capability is RequestCapabilityAvailable;

/// Whether a failed color request only means there is no color to give: the
/// item is missing, or its library's service needs a sign-in first. Any other
/// failure is shown.
bool noColorFailure(RequestFailure failure) =>
    failure.kind == RequestErrorKind.notFound || failure.needsServiceSignIn;
