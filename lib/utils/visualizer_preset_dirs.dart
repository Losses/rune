import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

/// The preset folders of the visualizer. The single folder of earlier
/// versions is moved into the list once.
Future<List<String>> loadVisualizerPresetDirs() async {
  final saved = await $settingsManager.getValue<List>(kVisualizerPresetDirsKey);
  final dirs = [
    for (final dir in saved ?? const [])
      if (dir is String && dir.isNotEmpty) dir,
  ];

  final old = await $settingsManager.getValue<String>(kVisualizerPresetDirKey);
  if (old != null) {
    if (dirs.isEmpty && old.isNotEmpty) {
      dirs.add(old);
      await saveVisualizerPresetDirs(dirs);
    }
    await $settingsManager.removeValue(kVisualizerPresetDirKey);
  }

  return dirs;
}

Future<void> saveVisualizerPresetDirs(List<String> dirs) {
  return $settingsManager.setValue(kVisualizerPresetDirsKey, dirs);
}
