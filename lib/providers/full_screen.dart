import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:get_storage/get_storage.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

class FullScreenProvider with ChangeNotifier, FullScreenListener {
  static const String storageKey = 'fullscreen_state';
  final GetStorage _storage = GetStorage();

  bool _isFullScreen = FullScreen.isFullScreen;
  bool get isFullScreen => _isFullScreen;

  FullScreenProvider() {
    _initFullScreenState();
    FullScreen.addListener(this);
  }

  Future<void> _initFullScreenState() async {
    await GetStorage.init();
  }

  Future<void> setFullScreen(bool enabled, {bool notify = true}) async {
    FullScreen.setFullScreen(enabled);
    _isFullScreen = enabled;
    _saveFullScreenState();
    if (notify) notifyListeners();
  }

  void _saveFullScreenState() {
    _storage.write(storageKey, _isFullScreen);
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
