import 'dart:io';

import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';

import 'rune_log.dart';

/// Checks if the current environment is a desktop environment.
bool get isDesktop {
  if (kIsWeb) return false;
  return [
    TargetPlatform.windows,
    TargetPlatform.linux,
    TargetPlatform.macOS,
  ].contains(defaultTargetPlatform);
}

/// The kind of device an iOS build runs on.
enum IosDevice { iPhone, iPad, mac }

IosDevice? _iosDevice;

/// The device kind of this iOS build, or null off iOS and before
/// [detectIosDevice] has run.
IosDevice? get iosDevice => _iosDevice;

/// Reads the device kind once at startup. Does nothing off iOS.
Future<void> detectIosDevice() async {
  if (!Platform.isIOS) return;
  try {
    final info = await DeviceInfoPlugin().iosInfo;
    _iosDevice = iosDeviceOf(model: info.model, onMac: info.isiOSAppOnMac);
  } catch (error) {
    warn$('Could not detect the device model: $error');
  }
}

/// Maps the iOS device [model] (UIDevice.model) and whether the app runs as an
/// iOS app on a Mac to a device kind.
IosDevice iosDeviceOf({required String model, required bool onMac}) {
  if (onMac) return IosDevice.mac;
  if (model.startsWith('iPad')) return IosDevice.iPad;
  return IosDevice.iPhone;
}
