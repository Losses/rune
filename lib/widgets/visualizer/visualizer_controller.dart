import 'package:flutter/foundation.dart';

import '../../bindings/bindings.dart';

const _presetTransitionSeconds = 1.7;

/// What the corner of the visualizer shows: the name of a preset that just
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
  bool _locked = false;
  bool _menuOpen = false;
  bool _autoSwitchPaused = false;

  VisualizerStatusResponse? status;
  // Why the texture could not be set up.
  String? startError;
  bool hasPresets = false;

  /// Set by the surface whenever a preset from a folder starts or a problem
  /// comes up; null while the default preset shows.
  final ValueNotifier<VisualizerLabel?> label = ValueNotifier(null);

  /// The user stopped the automatic preset switching (MilkDrop's Scroll Lock).
  bool get locked => _locked;

  /// A new surface starts with a new render thread, so the old state is gone.
  void attach() {
    _attached = true;
    _locked = false;
    _menuOpen = false;
    _autoSwitchPaused = false;
    status = null;
    startError = null;
    hasPresets = false;
    label.value = null;
  }

  void detach() => _attached = false;

  void next() {
    if (!_attached) return;
    NextVisualizerPresetRequest(
      transitionSeconds: _presetTransitionSeconds,
    ).sendSignalToRust();
  }

  void previous() {
    if (!_attached) return;
    PreviousVisualizerPresetRequest(
      transitionSeconds: _presetTransitionSeconds,
    ).sendSignalToRust();
  }

  void random() {
    if (!_attached) return;
    RandomVisualizerPresetRequest(
      transitionSeconds: _presetTransitionSeconds,
    ).sendSignalToRust();
  }

  void toggleLock() {
    _locked = !_locked;
    _syncAutoSwitchPaused();
  }

  void setMenuOpen(bool open) {
    _menuOpen = open;
    _syncAutoSwitchPaused();
  }

  // Rust waits with the automatic switching while the preset is locked or the
  // menu is open, and counts again from zero afterwards.
  void _syncAutoSwitchPaused() {
    if (!_attached) return;
    final paused = _locked || _menuOpen;
    if (paused == _autoSwitchPaused) return;
    _autoSwitchPaused = paused;
    SetVisualizerAutoSwitchPausedRequest(paused: paused).sendSignalToRust();
  }
}
