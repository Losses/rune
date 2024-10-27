import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import '../utils/settings_manager.dart';

final SettingsManager settingsManager = SettingsManager();

class FullScreenProvider with ChangeNotifier, FullScreenListener {
  static const String storageKey = 'fullscreen_state';

  bool _isFullScreen = FullScreen.isFullScreen;
  bool get isFullScreen => _isFullScreen;

  FullScreenProvider() {
    FullScreen.addListener(this);
  }

  Future<void> setFullScreen(bool enabled, {bool notify = true}) async {
    FullScreen.setFullScreen(enabled);
    _isFullScreen = enabled;
    _saveFullScreenState();
    if (notify) notifyListeners();
  }

  void _saveFullScreenState() {
    settingsManager.setValue(storageKey, _isFullScreen);
  }

  @override
  void onFullScreenChanged(bool enabled, SystemUiMode? systemUiMode) {
    _isFullScreen = enabled;
    _saveFullScreenState();
    notifyListeners();
  }

  @override
  void dispose() {
    FullScreen.removeListener(this);
    super.dispose();
  }
}
