import 'dart:io';
import 'dart:async';

import 'package:get_storage/get_storage.dart';
import 'package:path_provider/path_provider.dart';

import '../utils/storage_key_manager.dart';
import 'rune_log.dart';

const storageName = 'rune';

Future<String> getSettingsPath() async {
  if (Platform.isLinux || Platform.isWindows) {
    return (await getApplicationSupportDirectory()).path;
  }

  return (await getApplicationDocumentsDirectory()).path;
}

/// Returns [value] if it is a [T], or null as if the setting was never saved,
/// so that one stored value of the wrong type cannot stop Rune from starting.
T? castSetting<T>(String key, Object? value) {
  if (value is T?) return value;

  warn$("Ignoring setting $key: expected $T, found ${value.runtimeType}");
  return null;
}

class SettingsManager {
  static final SettingsManager _instance = SettingsManager._internal();
  factory SettingsManager() => _instance;

  late GetStorage _storage;
  bool _initialized = false;
  Future<void>? _initFuture;

  final Map<String, StreamController<dynamic>> _controllers = {};

  SettingsManager._internal() {
    _initFuture = _init();
  }

  Future<void> _init() async {
    if (_initialized) return;

    final path = await getSettingsPath();
    info$("Initializing config file at: $path");

    _storage = GetStorage(storageName, path);

    await _storage.initStorage;

    _initialized = true;
  }

  Future<T?> getValue<T>(String key) async {
    await _initFuture;
    String storageKey = StorageKeyManager.getStorageKey(key);
    return castSetting<T>(storageKey, _storage.read(storageKey));
  }

  Future<void> setValue<T>(String key, T value) async {
    await _initFuture;
    String storageKey = StorageKeyManager.getStorageKey(key);
    await _storage.write(storageKey, value);

    // Notify listeners if any
    if (_controllers.containsKey(storageKey)) {
      _controllers[storageKey]!.add(value);
    }
  }

  Future<void> removeValue<T>(String key) async {
    await _initFuture;
    String storageKey = StorageKeyManager.getStorageKey(key);
    await _storage.remove(storageKey);

    // Notify listeners if any
    if (_controllers.containsKey(storageKey)) {
      _controllers[storageKey]!.add(null);
    }
  }

  StreamSubscription listenValue<T>(
    String key,
    void Function(T? value) callback,
  ) {
    String storageKey = StorageKeyManager.getStorageKey(key);

    if (!_controllers.containsKey(storageKey)) {
      _controllers[storageKey] = StreamController<T?>.broadcast();
    }

    return (_controllers[storageKey]! as StreamController<T?>)
        .stream
        .listen(callback);
  }
}
