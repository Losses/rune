import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

/// How often the visualizer plays effects at the music's moments.
enum VisualizerEffectLevel { normal, more }

extension on VisualizerEffectLevel {
  /// The name Rust knows the level by.
  String get wire => this == VisualizerEffectLevel.more ? 'more' : 'default';
}

Future<VisualizerEffectLevel> loadVisualizerEffectLevel() async {
  final stored = await $settingsManager.getValue<String>(
    kVisualizerEffectLevelKey,
  );
  return stored == 'more'
      ? VisualizerEffectLevel.more
      : VisualizerEffectLevel.normal;
}

/// Saves the level and tells Rust, which is shared by all visualizers of the
/// app, so a running visualizer follows at once.
Future<void> saveVisualizerEffectLevel(VisualizerEffectLevel level) async {
  await $settingsManager.setValue(kVisualizerEffectLevelKey, level.wire);
  sendVisualizerEffectLevel(level);
}

void sendVisualizerEffectLevel(VisualizerEffectLevel level) {
  SetVisualizerEffectLevelRequest(level: level.wire).sendSignalToRust();
}
