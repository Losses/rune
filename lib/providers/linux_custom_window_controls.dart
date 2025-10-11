import 'dart:io';
import 'dart:async';

import 'package:flutter/foundation.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/linux_window_frame_manager.dart';

class LinuxCustomWindowControlsProvider extends ChangeNotifier {
  static final LinuxCustomWindowControlsProvider _instance =
      LinuxCustomWindowControlsProvider._internal();
  factory LinuxCustomWindowControlsProvider() => _instance;
  LinuxCustomWindowControlsProvider._internal();

  bool _enabled = false;
  bool _initialized = false;
  StreamSubscription? _subscription;

  bool get enabled => _enabled;
  bool get initialized => _initialized;

  // Allow synchronous initialization with preloaded value
  void initializeWithValue(bool value) {
    if (Platform.isLinux && !_initialized) {
      _enabled = value;
      _initialized = true;

      // Apply the frame setting immediately
      _applyFrameSetting(value);
    }
  }

  Future<void> initialize() async {
    if (_initialized) return;

    // Only set up listener on Linux
    if (Platform.isLinux) {
      // Load initial value only if not already initialized
      if (!_initialized) {
        final storedValue = await $settingsManager.getValue<bool>(kLinuxCustomWindowControlsKey);
        _enabled = storedValue ?? false;
        _initialized = true;
      }

      // Listen for changes
      _subscription = $settingsManager.listenValue<bool>(
        kLinuxCustomWindowControlsKey,
        (value) {
          final newValue = value ?? false;
          if (_enabled != newValue) {
            _enabled = newValue;
            notifyListeners();
          }
        },
      );
    } else {
      _initialized = true;
    }
  }

  Future<void> setEnabled(bool value) async {
    if (!Platform.isLinux) return;

    if (_enabled != value) {
      _enabled = value;
      await $settingsManager.setValue(kLinuxCustomWindowControlsKey, value);
      notifyListeners();

      // Apply the frame setting when changed
      _applyFrameSetting(value);
    }
  }

  void _applyFrameSetting(bool enabled) {
    if (Platform.isLinux) {
      try {
        LinuxWindowFrameManager.shared.setCustomFrame(enabled);
      } catch (e) {
        if (kDebugMode) {
          print('Failed to apply frame setting: $e');
        }
      }
    }
  }

  @override
  void dispose() {
    _subscription?.cancel();
    super.dispose();
  }
}