import 'package:get_storage/get_storage.dart';

import '../utils/storage_key_manager.dart';

class SettingsManager {
  static final SettingsManager _instance = SettingsManager._internal();
  factory SettingsManager() => _instance;

  final GetStorage _storage = GetStorage();
  bool _initialized = false;
  Future<void>? _initFuture;

  SettingsManager._internal() {
    _initFuture = _init();
  }

  Future<void> _init() async {
    if (!_initialized) {
      await GetStorage.init();
      _initialized = true;
    }
  }

  Future<T?> getValue<T>(String key) async {
    await _initFuture;
    String storageKey = StorageKeyManager.getStorageKey(key);
    return _storage.read<T>(storageKey);
  }

  Future<void> setValue<T>(String key, T value) async {
    await _initFuture;
    String storageKey = StorageKeyManager.getStorageKey(key);
    await _storage.write(storageKey, value);
  }
}
