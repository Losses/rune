import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'logger.dart';

/// The system's output volume and whether Rune may set it.
@immutable
class SystemVolumeState {
  const SystemVolumeState({required this.volume, required this.settable});

  /// From 0 to 1. A muted output reads as 0.
  final double volume;

  /// Whether the current output has a volume Rune can move. Without one, the control is shown
  /// disabled. On iOS it is always true: the system's own volume view moves the volume there,
  /// and nothing in Rune can tell whether it would work.
  final bool settable;

  @override
  bool operator ==(Object other) =>
      other is SystemVolumeState &&
      other.volume == volume &&
      other.settable == settable;

  @override
  int get hashCode => Object.hash(volume, settable);
}

/// The system volume of this device, which Rune's volume control stands for while a platform
/// host plays the track and has no volume of its own.
abstract class SystemVolume {
  /// The system volume on the platforms whose runner provides it, macOS, iOS and Android;
  /// `null` elsewhere, where no platform host plays and Rune's own volume always applies.
  static SystemVolume? forPlatform(TargetPlatform platform) =>
      !kIsWeb &&
          (platform == TargetPlatform.iOS ||
              platform == TargetPlatform.macOS ||
              platform == TargetPlatform.android)
      ? ChannelSystemVolume()
      : null;

  /// The view type of the runner's system volume view on iOS, which moves the system volume
  /// where Rune cannot.
  static const viewType = 'ci.not.rune/system_volume_view';

  /// The current state, or `null` when the runner cannot tell.
  Future<SystemVolumeState?> read();

  /// Sets the volume, from 0 to 1. Whether the runner did it.
  Future<bool> set(double volume);

  /// Every later change, whether Rune made it or a hardware key, the system or a device change.
  Stream<SystemVolumeState> get changes;
}

/// The runner's side of [SystemVolume], on macOS, iOS and Android.
class ChannelSystemVolume implements SystemVolume {
  ChannelSystemVolume({MethodChannel? channel})
    : _channel = channel ?? const MethodChannel(channelName) {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'changed') {
        final state = _parse(call.arguments);
        if (state != null) _changes.add(state);
      }
      return null;
    });
  }

  static const channelName = 'ci.not.rune/system_volume';

  final MethodChannel _channel;
  final _changes = StreamController<SystemVolumeState>.broadcast();

  static SystemVolumeState? _parse(Object? reply) {
    if (reply is! Map) return null;
    final volume = reply['volume'];
    final settable = reply['settable'];
    if (volume is! num || settable is! bool) return null;
    return SystemVolumeState(
      volume: volume.toDouble().clamp(0.0, 1.0),
      settable: settable,
    );
  }

  @override
  Future<SystemVolumeState?> read() async {
    try {
      return _parse(await _channel.invokeMethod<Object>('volume'));
    } on PlatformException catch (error) {
      logger.w('Reading the system volume failed: ${error.code}');
      return null;
    } on MissingPluginException {
      return null;
    }
  }

  @override
  Future<bool> set(double volume) async {
    try {
      await _channel.invokeMethod<void>('setVolume', volume);
      return true;
    } on PlatformException catch (error) {
      logger.w('Setting the system volume failed: ${error.code}');
      return false;
    } on MissingPluginException {
      return false;
    }
  }

  @override
  Stream<SystemVolumeState> get changes => _changes.stream;
}
