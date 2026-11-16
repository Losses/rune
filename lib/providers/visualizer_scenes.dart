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
import '../utils/api/request_failure.dart';

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
    required this.inputs,
  });

  final String path;
  final String name;
  final String group;
  final bool builtin;
  final bool enabled;
  final VisualizerSceneInputs inputs;

  bool get isCoverWall => path == kCoverWallScenePath;

  /// A scene the user imported from a scene file, which can be deleted.
  bool get imported => !builtin && !isCoverWall;

  VisualizerScene copyWith({bool? enabled, VisualizerSceneInputs? inputs}) =>
      VisualizerScene(
        path: path,
        name: name,
        group: group,
        builtin: builtin,
        enabled: enabled ?? this.enabled,
        inputs: inputs ?? this.inputs,
      );
}

/// Where the order and the unchecked scenes are kept between runs.
abstract class VisualizerSceneStore {
  Future<List<String>?> loadOrder();
  Future<List<String>?> loadDisabled();
  Future<void> save(List<String> order, List<String> disabled);
}

class _SettingsSceneStore implements VisualizerSceneStore {
  Future<List<String>?> _paths(String key) async {
    final saved = await $settingsManager.getValue<List>(key);
    if (saved == null) return null;
    return [
      for (final path in saved)
        if (path is String) path,
    ];
  }

  @override
  Future<List<String>?> loadOrder() => _paths(kVisualizerScenesKey);

  @override
  Future<List<String>?> loadDisabled() => _paths(kVisualizerDisabledScenesKey);

  @override
  Future<void> save(List<String> order, List<String> disabled) async {
    await $settingsManager.setValue(kVisualizerScenesKey, order);
    await $settingsManager.setValue(kVisualizerDisabledScenesKey, disabled);
  }
}

/// The scenes of the visualizer in the order the user arranged them, which
/// "next" goes through, and which of them are checked. The scenes come from Rust; the
/// rest is saved in the settings.
class VisualizerScenesProvider extends ChangeNotifier {
  List<VisualizerScene> _scenes = const [];
  // The saved order and unchecked scenes, also of scenes that are not listed
  // now (a scan that failed, or one that has not run yet): they keep their
  // place and their check until they are deleted.
  List<String> _order = const [];
  Set<String> _disabled = const {};
  bool _log = false;
  String? _lastScene;
  bool? _gpu;
  bool _loaded = false;
  bool _sceneShowing = false;
  VisualizerInputs _inputs = VisualizerInputs.none;
  Future<void>? _refreshing;
  bool _refreshAgain = false;
  Future<void> _applying = Future.value();
  final _firstList = Completer<void>();
  final VisualizerSceneStore _store;
  final void Function(SetVisualizerScenesRequest request) _sendScenes;
  StreamSubscription<VisualizerScenesChanged>? _scenesChanged;

  VisualizerScenesProvider()
    : _store = _SettingsSceneStore(),
      _sendScenes = _sendScenesToRust {
    if (isVisualizerSupported) {
      _listen(
        VisualizerScenesChanged.rustSignalStream.map((event) => event.message),
      );
      _load();
    }
  }

  /// A provider that only listens to [scenesChanged], with its own store and
  /// without Rust, the settings or the scene folder.
  @visibleForTesting
  VisualizerScenesProvider.listening({
    required Stream<VisualizerScenesChanged> scenesChanged,
    required VisualizerSceneStore store,
    required void Function(SetVisualizerScenesRequest request) sendScenes,
  }) : _store = store,
       _sendScenes = sendScenes {
    _listen(scenesChanged);
  }

  static void _sendScenesToRust(SetVisualizerScenesRequest request) =>
      request.sendSignalToRust();

  /// Whether the first list of scenes has arrived.
  @visibleForTesting
  bool get loaded => _loaded;

  /// Rust sends the list when it is ready and whenever it changes.
  void _listen(Stream<VisualizerScenesChanged> scenesChanged) {
    _scenesChanged = scenesChanged.listen((event) => _apply(event.presets));
  }

  @visibleForTesting
  VisualizerScenesProvider.forTest({
    required List<VisualizerScene> scenes,
    bool gpu = true,
  }) : _scenes = scenes,
       _gpu = gpu,
       _loaded = true,
       _store = _SettingsSceneStore(),
       _sendScenes = _sendScenesToRust;

  List<VisualizerScene> get scenes => _scenes
      .where(
        (scene) =>
            sceneSupportsVisualizerInputs(scene.inputs, _inputs) &&
            (_inputs != VisualizerInputs.spectrumOnly || !scene.isCoverWall),
      )
      .toList();

  VisualizerInputs get inputs => _inputs;

  void setInputs(VisualizerInputs inputs) {
    if (_inputs == inputs) return;
    _inputs = inputs;
    notifyListeners();
  }

  @visibleForTesting
  void applySupport(VisualizerSupportResponse support) {
    switch (support.support) {
      case VisualizerSupportSupported():
        _gpu = true;
      case VisualizerSupportUnsupported():
        _gpu = false;
    }
    setInputs(support.inputs);
    notifyListeners();
  }

  /// Whether the cover wall page runs the visualizer: some checked scene is
  /// not the cover wall. With only the cover wall checked the page shows the
  /// plain wall and the visualizer never starts. False until the scenes are
  /// loaded.
  bool get showVisualizer =>
      isVisualizerSupported &&
      _loaded &&
      _gpu == true &&
      scenes.any((scene) => scene.enabled && !scene.isCoverWall);

  /// Whether this machine has a GPU for the scenes: null until Rust has
  /// looked. Without one (no GPU, or only a software renderer that would draw
  /// on the CPU) the visualizer stays out of sight: the cover wall shows, and
  /// the settings leave its scenes out.
  bool? get gpu => _gpu;

  /// Whether the visualizer's diagnostics go to the Log page (its errors
  /// always do).
  bool get log => _log;

  /// Whether the scene the visualizer resumes is one other than the cover
  /// wall, as far as the app knows (the last one on screen).
  bool get resumesScene =>
      _lastScene != null && _lastScene != kCoverWallScenePath;

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
      // the plain wall. Signals sent before that wait for it. Rust sends the
      // list when it is ready; asking once covers a list that already is.
      SetVisualizerLogRequest(enabled: _log).sendSignalToRust();
      SetVisualizerPresetDirsRequest(paths: [dir]).sendSignalToRust();
      await refresh();
      await _firstList.future;
      // Scene files the system asked Rune to open; Rust is listening now.
      SceneFiles.opened.listen((file) async {
        final result = await importScene(file);
        await _dropCopy(file);
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
    final directory = await sceneDir();
    try {
      await awaitResponse(
        'ImportVisualizerSceneRequest',
        ImportVisualizerSceneResponse.rustSignalStream,
        timeout: const Duration(seconds: 30),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => ImportVisualizerSceneRequest(
          file: file,
          dir: directory,
          callId: callId,
        ).sendSignalToRust(),
      );
    } on RequestFailure catch (failure) {
      return VisualizerImport(name, error: failure.detail);
    } on TimeoutException {
      return VisualizerImport(name, error: 'No answer from the visualizer');
    }
    await refresh();
    return VisualizerImport(name);
  }

  /// Deletes the copy of an opened scene file that Android and iOS make to
  /// hand it over (`scene-imports/<id>/<file>` in the app's cache), once it is
  /// imported.
  Future<void> _dropCopy(String file) async {
    final parts = p.split(file);
    final at = parts.lastIndexOf('scene-imports');
    if (at < 0 || at + 2 >= parts.length) return;
    try {
      await Directory(p.joinAll(parts.take(at + 2))).delete(recursive: true);
    } catch (e) {
      error$('Failed to delete the copy of $file: $e');
    }
  }

  /// Deletes an imported scene.
  Future<String?> removeScene(VisualizerScene scene) async {
    if (!scene.imported) return null;
    final directory = await sceneDir();
    try {
      await awaitResponse(
        'RemoveVisualizerSceneRequest',
        RemoveVisualizerSceneResponse.rustSignalStream,
        timeout: const Duration(seconds: 10),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) => RemoveVisualizerSceneRequest(
          path: scene.path,
          dir: directory,
          callId: callId,
        ).sendSignalToRust(),
      );
    } on RequestFailure catch (failure) {
      return failure.detail;
    } on TimeoutException {
      return 'No answer from the visualizer';
    }
    _order = [..._order]..remove(scene.path);
    _disabled = {..._disabled}..remove(scene.path);
    await _save();
    await refresh();
    return null;
  }

  Future<void> _probeGpu() async {
    // Looking for a GPU can take a few seconds the first time.
    try {
      final support = (await awaitResponse(
        'VisualizerSupportRequest',
        VisualizerSupportResponse.rustSignalStream,
        timeout: const Duration(seconds: 20),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            VisualizerSupportRequest(callId: callId).sendSignalToRust(),
      )).message;
      applySupport(support);
    } on TimeoutException {
      _gpu = false;
    }
    notifyListeners();
  }

  /// Asks Rust for the scenes and reconciles them with the saved order and
  /// unchecked scenes. Scenes seen for the first time (all of them on the
  /// first run, then new built-in and imported scenes) are checked, at the end
  /// of the order; on the first run the cover wall comes first. Some scene
  /// always stays checked: the cover wall if there is none. Calls that overlap
  /// share one run after the current one.
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
    final List<VisualizerPresetEntry> listed;
    try {
      listed = (await awaitResponse(
        'ListVisualizerPresetsRequest',
        ListVisualizerPresetsResponse.rustSignalStream,
        timeout: const Duration(seconds: 2),
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (callId) =>
            ListVisualizerPresetsRequest(callId: callId).sendSignalToRust(),
      )).message.presets;
    } on TimeoutException {
      return;
    }

    await _apply(listed);
  }

  /// Applies a list of scenes, one at a time in the order they arrive.
  Future<void> _apply(List<VisualizerPresetEntry> listed) =>
      _applying = _applying.then((_) => _applyListed(listed));

  Future<void> _applyListed(List<VisualizerPresetEntry> listed) async {
    // The settings are read once; after that the lists here are newer.
    final storedOrder = _loaded ? _order : await _store.loadOrder();
    final storedDisabled = _loaded ? _disabled : await _store.loadDisabled();
    final byPath = {for (final entry in listed) entry.path: entry};
    final (order, disabled) = reconcileScenes(
      listed: listed.map((entry) => entry.path).toList(),
      storedOrder: storedOrder,
      storedDisabled: storedDisabled,
    );

    _order = order;
    _disabled = disabled;
    _scenes = [
      for (final path in order)
        if (byPath[path] case final entry?)
          VisualizerScene(
            path: path,
            name: entry.name,
            group: entry.group,
            builtin: entry.builtin,
            enabled: !disabled.contains(path),
            inputs: entry.inputs,
          ),
    ];
    _loaded = true;
    if (!_firstList.isCompleted) _firstList.complete();
    await _save();
    notifyListeners();
    _sendChoice();
  }

  @override
  void dispose() {
    _scenesChanged?.cancel();
    _imports.close();
    super.dispose();
  }

  Future<void> _save() => _store.save(_order, _disabled.toList());

  void _sendChoice() {
    _sendScenes(
      SetVisualizerScenesRequest(
        order: _order,
        disabled: _disabled.toList(),
        last: _lastScene,
      ),
    );
  }

  void _placeListed() {
    _order = placeArranged(_order, _scenes.map((scene) => scene.path).toList());
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
    _placeListed();
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
    _disabled = enabled ? ({..._disabled}..remove(path)) : {..._disabled, path};
    _save();
    notifyListeners();
    _sendChoice();
  }
}

bool sceneSupportsVisualizerInputs(
  VisualizerSceneInputs needs,
  VisualizerInputs available,
) => switch (available) {
  VisualizerInputs.none => false,
  VisualizerInputs.spectrumOnly => needs == VisualizerSceneInputs.spectrum,
  VisualizerInputs.full => true,
};

/// The order of the scenes and the unchecked ones, from what was saved
/// (`storedOrder` is null on the first run) and the scenes Rust lists now.
/// Saved scenes that are not listed keep their place and their check; listed
/// ones seen for the first time are checked, at the end, the cover wall first
/// on the first run. Some listed scene always stays checked: the cover wall,
/// or else the first in the order.
(List<String>, Set<String>) reconcileScenes({
  required List<String> listed,
  required Iterable<String>? storedOrder,
  required Iterable<String>? storedDisabled,
}) {
  final order = <String>[];
  void add(String path) {
    if (!order.contains(path)) order.add(path);
  }

  if (storedOrder == null && listed.contains(kCoverWallScenePath)) {
    add(kCoverWallScenePath);
  }
  storedOrder?.forEach(add);
  listed.forEach(add);
  final disabled = {...?storedDisabled};
  if (listed.isNotEmpty && listed.every(disabled.contains)) {
    disabled.remove(
      listed.contains(kCoverWallScenePath)
          ? kCoverWallScenePath
          : order.firstWhere(listed.contains),
    );
  }
  return (order, disabled);
}

/// `order` with the scenes of `arranged` in their new order, in the places
/// they had: a scene that is not in `arranged` (not listed now) keeps its
/// place among them.
List<String> placeArranged(List<String> order, List<String> arranged) {
  final listed = arranged.toSet();
  final next = arranged.iterator;
  return [
    for (final path in order)
      if (listed.contains(path) && next.moveNext()) next.current else path,
  ];
}

/// How importing a scene file went: [error] says why it is not a scene.
@immutable
class VisualizerImport {
  const VisualizerImport(this.name, {this.error});

  /// The scene's name, from the file's.
  final String name;
  final String? error;
}
