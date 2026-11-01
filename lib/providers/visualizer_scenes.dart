import 'dart:io';
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:path/path.dart' as p;
import 'package:path_provider/path_provider.dart';

import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/rune_log.dart';
import '../utils/scene_files.dart';
import '../utils/visualizer_supported.dart';

/// The cover art wall as a scene. Rust lists it with the others and draws
/// nothing while it is the scene on screen; the app shows the wall instead.
const kCoverWallScenePath = 'app:cover-wall';

/// A scene of the visualizer, with whether it may be picked.
@immutable
class VisualizerScene {
  const VisualizerScene({
    required this.path,
    required this.name,
    required this.group,
    required this.builtin,
    required this.enabled,
  });

  final String path;
  final String name;
  final String group;
  final bool builtin;
  final bool enabled;

  bool get isCoverWall => path == kCoverWallScenePath;

  /// A scene the user imported from a scene file, which can be deleted.
  bool get imported => !builtin && !isCoverWall;

  VisualizerScene copyWith({bool? enabled}) => VisualizerScene(
    path: path,
    name: name,
    group: group,
    builtin: builtin,
    enabled: enabled ?? this.enabled,
  );
}

/// The scenes of the visualizer in the order the user arranged them, which
/// "next" goes through, and which of them are checked. The scenes come from Rust; the
/// rest is saved in the settings.
class VisualizerScenesProvider extends ChangeNotifier {
  List<VisualizerScene> _scenes = const [];
  bool _log = false;
  String? _lastScene;
  bool? _gpu;
  String? _gpuProblem;
  bool _loaded = false;
  bool _sceneShowing = false;
  Future<void>? _refreshing;
  bool _refreshAgain = false;

  VisualizerScenesProvider() {
    if (isVisualizerSupported) _load();
  }

  List<VisualizerScene> get scenes => _scenes;

  /// Whether the cover wall page runs the visualizer: some checked scene is
  /// not the cover wall. With only the cover wall checked the page shows the
  /// plain wall and the visualizer never starts. False until the scenes are
  /// loaded.
  bool get showVisualizer =>
      isVisualizerSupported &&
      _loaded &&
      _gpu == true &&
      _scenes.any((scene) => scene.enabled && !scene.isCoverWall);

  /// Whether this machine has a GPU for the scenes: null until Rust has
  /// looked. Without one (no GPU, or only a software renderer that would draw
  /// on the CPU) only the cover wall can be shown.
  bool? get gpu => _gpu;

  /// Why there is no GPU for the scenes, as Rust says it (in English).
  String? get gpuProblem => _gpuProblem;

  /// Whether the visualizer's diagnostics go to the Log page (its errors
  /// always do).
  bool get log => _log;

  /// The path of the scene on screen, kept so that the visualizer resumes it
  /// after a restart (within a run, Rust remembers it).
  void setLastScene(String path) {
    if (path == _lastScene) return;
    _lastScene = path;
    $settingsManager.setValue(kVisualizerLastSceneKey, path);
  }

  void setLog(bool log) {
    if (log == _log) return;
    _log = log;
    $settingsManager.setValue(kVisualizerLogKey, log);
    notifyListeners();
    SetVisualizerLogRequest(enabled: log).sendSignalToRust();
  }

  /// Whether a scene other than the cover wall is on screen. Set by the
  /// visualizer surface; scenes draw on black.
  bool get sceneShowing => _sceneShowing;
  set sceneShowing(bool value) {
    if (value == _sceneShowing) return;
    _sceneShowing = value;
    notifyListeners();
  }

  /// The folder imported scenes are unpacked into, one folder each, made if
  /// missing.
  static Future<String> sceneDir() async {
    final support = await getApplicationSupportDirectory();
    final dir = Directory(p.join(support.path, 'visualizer-scenes'));
    await dir.create(recursive: true);
    return dir.path;
  }

  Future<void> _load() async {
    try {
      final dir = await sceneDir();
      _log = await $settingsManager.getValue<bool>(kVisualizerLogKey) ?? false;
      _lastScene = await $settingsManager.getValue<String>(
        kVisualizerLastSceneKey,
      );
      // Rust answers once the library is open, which can take a while (or
      // wait for the user to pick one); until then the cover wall page shows
      // the plain wall. Signals sent before that are lost, so they are sent
      // again with every attempt.
      while (!_loaded) {
        SetVisualizerLogRequest(enabled: _log).sendSignalToRust();
        SetVisualizerPresetDirsRequest(paths: [dir]).sendSignalToRust();
        await refresh();
        if (!_loaded) await Future.delayed(const Duration(seconds: 3));
      }
      // Scene files the system asked Rune to open; Rust is listening now.
      SceneFiles.opened.listen((file) async {
        final result = await importScene(file);
        _imports.add(result);
      });
      await _probeGpu();
    } catch (e) {
      error$('Failed to load the visualizer scenes: $e');
    }
  }

  final _imports = StreamController<VisualizerImport>.broadcast();

  /// The outcome of every import of a scene file the system asked Rune to
  /// open, for a notification.
  Stream<VisualizerImport> get imports => _imports.stream;

  /// Unpacks the scene file [file] into [sceneDir] (a scene of the same name
  /// is replaced) and lists it, checked, at the end of the order.
  Future<VisualizerImport> importScene(String file) async {
    final name = p.basenameWithoutExtension(file);
    final response = ImportVisualizerSceneResponse.rustSignalStream
        .where((signal) => signal.message.file == file)
        .first
        .timeout(const Duration(seconds: 30));
    ImportVisualizerSceneRequest(
      file: file,
      dir: await sceneDir(),
    ).sendSignalToRust();
    try {
      final message = (await response).message;
      if (message.error != null) {
        return VisualizerImport(name, error: message.error);
      }
    } on TimeoutException {
      return VisualizerImport(name, error: 'No answer from the visualizer');
    }
    await refresh();
    return VisualizerImport(name);
  }

  /// Deletes an imported scene.
  Future<String?> removeScene(VisualizerScene scene) async {
    if (!scene.imported) return null;
    final response = RemoveVisualizerSceneResponse.rustSignalStream
        .where((signal) => signal.message.path == scene.path)
        .first
        .timeout(const Duration(seconds: 10));
    RemoveVisualizerSceneRequest(
      path: scene.path,
      dir: await sceneDir(),
    ).sendSignalToRust();
    try {
      final error = (await response).message.error;
      if (error != null) return error;
    } on TimeoutException {
      return 'No answer from the visualizer';
    }
    await refresh();
    return null;
  }

  Future<void> _probeGpu() async {
    // Looking for a GPU can take a few seconds the first time.
    final response = VisualizerSupportResponse.rustSignalStream.first.timeout(
      const Duration(seconds: 20),
    );
    VisualizerSupportRequest().sendSignalToRust();
    try {
      final support = (await response).message;
      _gpu = support.supported;
      _gpuProblem = support.problem;
    } on TimeoutException {
      _gpu = false;
      _gpuProblem = 'No answer from the GPU probe';
    }
    notifyListeners();
  }

  /// Asks Rust for the scenes and reconciles them with the saved order and
  /// unchecked scenes. Scenes seen for the first time (all of them on the
  /// first run, then new built-in scenes and those of a new folder) are
  /// checked, at the end of the order; on the first run the cover wall comes
  /// first. Calls that overlap share one run after the current one.
  Future<void> refresh() {
    final running = _refreshing;
    if (running != null) {
      _refreshAgain = true;
      return running;
    }
    return _refreshing = _runRefresh();
  }

  Future<void> _runRefresh() async {
    try {
      while (true) {
        await _refreshOnce();
        if (!_refreshAgain) break;
        _refreshAgain = false;
      }
    } finally {
      _refreshing = null;
    }
  }

  Future<void> _refreshOnce() async {
    final response = ListVisualizerPresetsResponse.rustSignalStream.first
        .timeout(const Duration(seconds: 2));
    ListVisualizerPresetsRequest().sendSignalToRust();
    final List<VisualizerPresetEntry> listed;
    try {
      listed = (await response).message.presets;
    } on TimeoutException {
      return;
    }

    final storedOrder = await _loadPaths(kVisualizerScenesKey);
    final storedDisabled = await _loadPaths(kVisualizerDisabledScenesKey);
    final byPath = {for (final entry in listed) entry.path: entry};

    final order = <String>[];
    if (storedOrder == null && byPath.containsKey(kCoverWallScenePath)) {
      order.add(kCoverWallScenePath);
    }
    order.addAll((storedOrder ?? const []).where(byPath.containsKey));
    for (final entry in listed) {
      if (!order.contains(entry.path)) order.add(entry.path);
    }
    final disabled = (storedDisabled ?? const []).toSet();

    _scenes = [
      for (final path in order)
        VisualizerScene(
          path: path,
          name: byPath[path]!.name,
          group: byPath[path]!.group,
          builtin: byPath[path]!.builtin,
          enabled: !disabled.contains(path),
        ),
    ];
    _loaded = true;
    await _save();
    notifyListeners();
    _sendChoice();
  }

  Future<List<String>?> _loadPaths(String key) async {
    final saved = await $settingsManager.getValue<List>(key);
    if (saved == null) return null;
    return [
      for (final path in saved)
        if (path is String) path,
    ];
  }

  Future<void> _save() async {
    await $settingsManager.setValue(
      kVisualizerScenesKey,
      _scenes.map((scene) => scene.path).toList(),
    );
    await $settingsManager.setValue(
      kVisualizerDisabledScenesKey,
      _disabledPaths(),
    );
  }

  List<String> _disabledPaths() => [
    for (final scene in _scenes)
      if (!scene.enabled) scene.path,
  ];

  void _sendChoice() {
    SetVisualizerScenesRequest(
      order: _scenes.map((scene) => scene.path).toList(),
      disabled: _disabledPaths(),
      last: _lastScene,
    ).sendSignalToRust();
  }

  /// Moves a scene, with the semantics of `ReorderableColumn.onReorder`.
  void reorder(int oldIndex, int newIndex) {
    if (oldIndex < 0 ||
        oldIndex >= _scenes.length ||
        newIndex < 0 ||
        newIndex >= _scenes.length) {
      return;
    }
    final scenes = [..._scenes];
    scenes.insert(newIndex, scenes.removeAt(oldIndex));
    _scenes = scenes;
    _save();
    notifyListeners();
    _sendChoice();
  }

  /// Checks or unchecks a scene. The last checked scene cannot be unchecked.
  void setEnabled(String path, bool enabled) {
    final index = _scenes.indexWhere((scene) => scene.path == path);
    if (index < 0 || _scenes[index].enabled == enabled) return;
    if (!enabled && _scenes.where((scene) => scene.enabled).length <= 1) {
      return;
    }
    final scenes = [..._scenes];
    scenes[index] = scenes[index].copyWith(enabled: enabled);
    _scenes = scenes;
    _save();
    notifyListeners();
    _sendChoice();
  }
}

/// How importing a scene file went: [error] says why it is not a scene.
@immutable
class VisualizerImport {
  const VisualizerImport(this.name, {this.error});

  /// The scene's name, from the file's.
  final String name;
  final String? error;
}
