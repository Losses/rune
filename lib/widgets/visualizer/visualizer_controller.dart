import 'dart:async';

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
  // The presets of the folders, as of the last time the menu opened.
  List<VisualizerPresetEntry> presets = const [];

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
    presets = const [];
    label.value = null;
  }

  void detach() => _attached = false;

  // A locked preset is one the user is watching on purpose, so changing it by
  // hand cuts straight to the new one instead of blending.
  double get _transitionSeconds => _locked ? 0 : _presetTransitionSeconds;

  void next() {
    if (!_attached) return;
    NextVisualizerPresetRequest(
      transitionSeconds: _transitionSeconds,
    ).sendSignalToRust();
  }

  void previous() {
    if (!_attached) return;
    PreviousVisualizerPresetRequest(
      transitionSeconds: _transitionSeconds,
    ).sendSignalToRust();
  }

  void random() {
    if (!_attached) return;
    RandomVisualizerPresetRequest(
      transitionSeconds: _transitionSeconds,
    ).sendSignalToRust();
  }

  void loadPreset(String path) {
    if (!_attached) return;
    LoadVisualizerPresetRequest(
      path: path,
      transitionSeconds: _transitionSeconds,
    ).sendSignalToRust();
  }

  /// Asks Rust for the presets of the folders, which a menu lists. Keeps the
  /// earlier list if Rust does not answer.
  Future<void> refreshPresets() async {
    if (!_attached) return;
    final response = ListVisualizerPresetsResponse.rustSignalStream.first
        .timeout(const Duration(seconds: 1));
    ListVisualizerPresetsRequest().sendSignalToRust();
    try {
      presets = (await response).message.presets;
    } on TimeoutException {
      // The menu shows the list it has.
    }
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
