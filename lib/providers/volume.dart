// volume_provider.dart
import 'dart:async';

import 'package:rinf/rinf.dart';
import 'package:flutter/foundation.dart';

import '../utils/settings_manager.dart';
import '../utils/system_volume.dart';
import '../bindings/bindings.dart';
import 'renderer_attachment.dart';

/// Whether a platform host with no volume of its own plays what this device's own player plays:
/// the latest report and the ones that follow.
class HostedSignal {
  const HostedSignal({required this.latest, required this.changes});

  /// What the hub reports from the live signals.
  factory HostedSignal.fromHub() => HostedSignal.fromReports(
    latest: PlayerHostChanged.latestRustSignal?.message,
    changes: PlayerHostChanged.rustSignalStream.map((event) => event.message),
  );

  /// From the hub's reports of the host that plays this device's track. A host with a volume of
  /// its own does not count: it plays at Rune's volume, as this device's own player does.
  factory HostedSignal.fromReports({
    required PlayerHostChanged? latest,
    required Stream<PlayerHostChanged> changes,
  }) => HostedSignal(
    latest: _withoutOwnVolume(latest),
    changes: changes.map(_withoutOwnVolume),
  );

  static bool _withoutOwnVolume(PlayerHostChanged? report) =>
      report != null && report.hosted && !report.ownVolume;

  final bool latest;
  final Stream<bool> changes;
}

/// Rune's volume, or the system volume while a platform host with no volume of its own plays
/// this device's track.
///
/// A platform host on macOS or iOS may play with no volume of its own, so the volume control
/// moves the system volume then. That is only so when this device's own player is the current
/// one and the host plays the track; Rune's stored volume is left alone, and the system volume
/// stays where it is once the mode ends. A host with a volume of its own plays at Rune's volume,
/// as this device's own player does.
class VolumeProvider with ChangeNotifier {
  static const String _volumeSettingsKey = 'volume_level';

  /// How long the system's reports of a volume are ignored after Rune set one: the report of an
  /// earlier set can land after a later one and would move the slider back while it is dragged.
  static const systemEchoWindow = Duration(milliseconds: 250);

  final Future<double?> Function() _readStoredVolume;
  final Future<void> Function(double volume) _writeStoredVolume;
  final void Function(VolumeRequest request) _sendVolumeRequest;
  final SystemVolume? _systemVolume;
  final RendererAttachmentProvider? _renderers;
  final DateTime Function() _clock;
  final bool _systemVolumeView;
  late StreamSubscription<RustSignalPack<VolumeResponse>> _subscription;
  StreamSubscription<bool>? _hostSubscription;
  StreamSubscription<SystemVolumeState>? _systemSubscription;

  double _volume = 1;
  bool _hosted = false;
  bool _systemMode = false;
  bool _disposed = false;

  /// The system's volume and what it allows; `null` until the runner has answered, so that
  /// nothing of Rune's own volume shows for it.
  SystemVolumeState? _system;
  DateTime? _lastSystemSet;
  SystemVolumeState? _deferredSystem;
  Timer? _deferTimer;

  /// The volume to show: the system's while [usesSystemVolume] (nothing until it is known),
  /// else Rune's.
  double get volume => _systemMode ? (_system?.volume ?? 0) : _volume;

  /// Whether the volume is the system's, because a platform host with no volume of its own plays
  /// this device's track.
  bool get usesSystemVolume => _systemMode;

  /// Whether the system's own volume view stands in for the slider: on iOS, which has no call
  /// to set the system volume.
  bool get showsSystemVolumeView => _systemMode && _systemVolumeView;

  /// Whether the volume can be changed: not before the system volume is known, nor when the
  /// output has no volume to move.
  bool get adjustable => !_systemMode || (_system?.settable ?? false);

  Timer? _debounceTimer;
  int _userVolumeGeneration = 0;

  /// [platform] is the one Rune runs on. [systemVolume] and [hosted] default to the runner's
  /// and the hub's, and the system volume applies on macOS, iOS and Android only.
  VolumeProvider({
    Future<double?> Function()? readStoredVolume,
    Future<void> Function(double volume)? writeStoredVolume,
    void Function(VolumeRequest request)? sendVolumeRequest,
    SystemVolume? systemVolume,
    RendererAttachmentProvider? renderers,
    HostedSignal? hosted,
    TargetPlatform? platform,
    DateTime Function()? clock,
  }) : _readStoredVolume = readStoredVolume ?? _readVolumeFromSettings,
       _writeStoredVolume = writeStoredVolume ?? _writeVolumeToSettings,
       _sendVolumeRequest = sendVolumeRequest ?? _sendVolumeToRust,
       _renderers = renderers,
       _clock = clock ?? DateTime.now,
       _systemVolumeView =
           (platform ?? defaultTargetPlatform) == TargetPlatform.iOS,
       _systemVolume = _systemVolumeOn(platform ?? defaultTargetPlatform)
           ? (systemVolume ??
                 SystemVolume.forPlatform(platform ?? defaultTargetPlatform))
           : null {
    _initVolume();
    _subscription = VolumeResponse.rustSignalStream.listen(
      _handleVolumeResponse,
    );
    if (_systemVolume != null) {
      final signal = hosted ?? HostedSignal.fromHub();
      _hosted = signal.latest;
      _hostSubscription = signal.changes.listen((hosted) {
        _hosted = hosted;
        _updateMode();
      });
      // A renderer attached is the backstop: the hub says no host plays then too.
      _renderers?.addListener(_updateMode);
      _updateMode();
    }
  }

  static bool _systemVolumeOn(TargetPlatform platform) =>
      !kIsWeb &&
      (platform == TargetPlatform.iOS ||
          platform == TargetPlatform.macOS ||
          platform == TargetPlatform.android);

  void _updateMode() {
    final systemMode =
        _systemVolume != null && _hosted && _renderers?.renderer == null;
    if (systemMode == _systemMode) return;
    _systemMode = systemMode;
    if (systemMode) {
      _system = null;
      _systemSubscription = _systemVolume.changes.listen(_systemChanged);
      _readSystemVolume();
    } else {
      _systemSubscription?.cancel();
      _systemSubscription = null;
      _dropDeferred();
    }
    notifyListeners();
  }

  void _systemChanged(SystemVolumeState state) {
    final lastSet = _lastSystemSet;
    final inWindow =
        lastSet != null && _clock().difference(lastSet) < systemEchoWindow;
    if (!inWindow) {
      // A report held back in the window is older than this one.
      _dropDeferred();
      _applySystem(state);
      return;
    }
    // Maybe the echo of a set, maybe a key pressed or a device rounding the value: the latest
    // report inside the window is applied once the window closes.
    _deferredSystem = state;
    _deferTimer?.cancel();
    _deferTimer = Timer(
      lastSet.add(systemEchoWindow).difference(_clock()),
      _applyDeferred,
    );
  }

  void _applyDeferred() {
    final state = _deferredSystem;
    _dropDeferred();
    if (state != null && _systemMode) _applySystem(state);
  }

  void _dropDeferred() {
    _deferTimer?.cancel();
    _deferTimer = null;
    _deferredSystem = null;
  }

  void _applySystem(SystemVolumeState state) {
    _system = state;
    notifyListeners();
  }

  Future<void> _readSystemVolume() async {
    final state = await _systemVolume!.read();
    // The mode may have ended, or the provider gone, while the runner answered.
    if (_disposed || !_systemMode || state == null) return;
    _system = state;
    notifyListeners();
  }

  Future<void> _initVolume() async {
    final generation = _userVolumeGeneration;
    final storedVolume = await _readStoredVolume();
    if (generation != _userVolumeGeneration) return;
    if (storedVolume != null) {
      _updateVolume(storedVolume, notify: false, save: false);
      _sendVolumeRequest(VolumeRequest(volume: storedVolume));
    }
  }

  static Future<double?> _readVolumeFromSettings() =>
      SettingsManager().getValue<double>(_volumeSettingsKey);

  static Future<void> _writeVolumeToSettings(double volume) =>
      SettingsManager().setValue(_volumeSettingsKey, volume);

  static void _sendVolumeToRust(VolumeRequest request) {
    request.sendSignalToRust();
  }

  void _handleVolumeResponse(RustSignalPack<VolumeResponse> signal) {
    _updateVolume(signal.message.volume);
  }

  void _updateVolume(double newVolume, {bool notify = true, bool save = true}) {
    _volume = newVolume;
    if (save) _saveVolume();
    if (notify) notifyListeners();
  }

  Future<void> _saveVolume() async {
    await _writeStoredVolume(_volume);
  }

  void updateVolume(double volume) {
    if (_systemMode) {
      // Neither Rune's volume nor the hub is involved: the system volume is the player's.
      // The system's own volume view moves it on iOS; nothing here can.
      if (!adjustable || _systemVolumeView) return;
      _system = SystemVolumeState(volume: volume, settable: true);
      _lastSystemSet = _clock();
      // The set is newer than what was held back.
      _dropDeferred();
      notifyListeners();
      _setSystemVolume(volume);
      return;
    }
    _userVolumeGeneration++;
    _updateVolume(volume);

    // Cancel the previous timer if it exists
    _debounceTimer?.cancel();

    // Start a new timer
    _debounceTimer = Timer(Duration(milliseconds: 200), () {
      _sendVolumeRequest(VolumeRequest(volume: volume));
    });
  }

  Future<void> _setSystemVolume(double volume) async {
    if (await _systemVolume!.set(volume)) return;
    // Not set: what the slider shows is not so, so it shows what the system says again.
    _lastSystemSet = null;
    _dropDeferred();
    await _readSystemVolume();
  }

  @override
  void dispose() {
    _disposed = true;
    _subscription.cancel();
    _hostSubscription?.cancel();
    _systemSubscription?.cancel();
    _deferTimer?.cancel();
    _renderers?.removeListener(_updateMode);
    _debounceTimer?.cancel();
    super.dispose();
  }
}
