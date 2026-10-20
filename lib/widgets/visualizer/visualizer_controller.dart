import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';

const _presetTransitionSeconds = 1.7;

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

  /// The preset name, or what is wrong, followed by the frame rate and the
  /// transport.
  String infoText(AppLocalizations s) {
    final startError = this.startError;
    if (startError != null) return startError;

    final status = this.status;
    if (status == null) return '';
    final error = status.lastError;
    return [
      error ?? status.presetName,
      if (status.running) ...[
        '${status.measuredFps.toStringAsFixed(0)} fps',
        // The transport is a diagnostic: "iosurface" means no copies on the
        // way to the screen.
        if (status.transport.isNotEmpty) status.transport,
      ],
    ].where((part) => part.isNotEmpty).join(' · ');
  }
}
