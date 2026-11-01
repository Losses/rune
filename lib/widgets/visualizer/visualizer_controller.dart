import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../bindings/bindings.dart';

/// The commands and the state of a [VisualizerSurface] for the widgets around
/// it, such as a context menu and keyboard shortcuts. The commands do nothing
/// while no surface is attached.
class VisualizerController {
  bool _attached = false;

  /// A problem of the visualizer, set by the surface (the texture could not be
  /// set up, a scene failed); null while there is none.
  final ValueNotifier<String?> problem = ValueNotifier(null);

  /// The path of the scene on screen, from the status; null before the
  /// visualizer runs.
  final ValueNotifier<String?> scenePath = ValueNotifier(null);

  /// Whether the page waits in the dark for the scene the visualizer begins
  /// with, rather than showing the cover wall until it comes: from when a
  /// surface attaches to resume a scene until a scene shows, a problem comes or
  /// [waitLimit] has passed.
  final ValueNotifier<bool> waiting = ValueNotifier(false);

  /// How long the page waits for the first scene before it shows the cover
  /// wall, for a visualizer that is slow to start or does not.
  static const waitLimit = Duration(seconds: 2);

  Timer? _waitTimer;

  /// A new surface starts with a new render thread, so the old state is gone.
  /// [resumesScene] tells that the visualizer resumes a scene other than the
  /// cover wall, which the page then waits for.
  void attach({bool resumesScene = false}) {
    _attached = true;
    problem.value = null;
    scenePath.value = null;
    _waitTimer?.cancel();
    waiting.value = resumesScene;
    if (resumesScene) _waitTimer = Timer(waitLimit, stopWaiting);
  }

  /// A scene showed or a problem came: the page waits no longer.
  void stopWaiting() {
    _waitTimer?.cancel();
    _waitTimer = null;
    waiting.value = false;
  }

  void detach() {
    _attached = false;
    stopWaiting();
  }

  void next() {
    if (!_attached) return;
    NextVisualizerPresetRequest().sendSignalToRust();
  }

  void previous() {
    if (!_attached) return;
    PreviousVisualizerPresetRequest().sendSignalToRust();
  }

  void random() {
    if (!_attached) return;
    RandomVisualizerPresetRequest().sendSignalToRust();
  }

  void loadPreset(String path) {
    if (!_attached) return;
    LoadVisualizerPresetRequest(path: path).sendSignalToRust();
  }
}
