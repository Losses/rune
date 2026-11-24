class StorageKeyManager {
  static const String defaultExtensionName = 'ci.not.rune';
  static String? _profile;

  static void initialize(String? profile) {
    _profile = profile ?? 'default';
  }

  static String getStorageKey(String settingsKey, {String? extensionName}) {
    final extension = extensionName ?? defaultExtensionName;
    return '$extension#$_profile:$settingsKey';
  }
}
