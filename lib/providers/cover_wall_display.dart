import 'package:flutter/foundation.dart';

import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/visualizer_supported.dart';

enum CoverWallDisplay { covers, visualizer }

/// What the cover wall page shows behind the playing track.
class CoverWallDisplayProvider extends ChangeNotifier {
  CoverWallDisplay _display = CoverWallDisplay.covers;

  CoverWallDisplayProvider() {
    _load();
  }

  CoverWallDisplay get display => _display;

  bool _presetsLoaded = false;

  /// Whether the visualizer is running, drawing a scene (which draws on
  /// black).
  bool get presetsLoaded => _presetsLoaded;
  set presetsLoaded(bool value) {
    if (value == _presetsLoaded) return;
    _presetsLoaded = value;
    notifyListeners();
  }

  /// The visualizer is only shown where the platform supports it.
  bool get showVisualizer =>
      _display == CoverWallDisplay.visualizer && isVisualizerSupported;

  Future<void> _load() async {
    final stored = await $settingsManager.getValue<String>(
      kCoverWallDisplayKey,
    );
    final display = stored == CoverWallDisplay.visualizer.name
        ? CoverWallDisplay.visualizer
        : CoverWallDisplay.covers;
    if (display == _display) return;
    _display = display;
    notifyListeners();
  }

  Future<void> setDisplay(CoverWallDisplay display) async {
    if (display == _display) return;
    _display = display;
    notifyListeners();
    await $settingsManager.setValue(kCoverWallDisplayKey, display.name);
  }
}
