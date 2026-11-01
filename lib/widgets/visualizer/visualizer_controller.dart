import 'package:flutter/foundation.dart';

import '../../bindings/bindings.dart';

/// What the corner of the visualizer shows: the name of a scene that just
/// started, or a problem.
@immutable
class VisualizerLabel {
  const VisualizerLabel(this.text, {this.isError = false});

  final String text;
  final bool isError;
}

/// The commands and the state of a [VisualizerSurface] for the widgets around
/// it, such as a context menu and keyboard shortcuts. The commands do nothing
/// while no surface is attached.
class VisualizerController {
  bool _attached = false;

  VisualizerStatusResponse? status;
  // Why the texture could not be set up.
  String? startError;
  bool hasPresets = false;

  /// Set by the surface whenever a scene from a folder starts or a problem
  /// comes up; null while the built-in scene shows.
  final ValueNotifier<VisualizerLabel?> label = ValueNotifier(null);

  /// The path of the scene on screen, from the status; null before the
  /// visualizer runs.
  final ValueNotifier<String?> scenePath = ValueNotifier(null);

  /// A new surface starts with a new render thread, so the old state is gone.
  void attach() {
    _attached = true;
    status = null;
    startError = null;
    hasPresets = false;
    label.value = null;
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
