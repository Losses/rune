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

  /// A new surface starts with a new render thread, so the old state is gone.
  void attach() {
    _attached = true;
    problem.value = null;
    scenePath.value = null;
  }

  void detach() => _attached = false;

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
