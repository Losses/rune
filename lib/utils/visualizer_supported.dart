import 'dart:io';

/// Whether the platform has the texture bridge to the Rust visualizer.
bool get isVisualizerSupported {
  return Platform.isMacOS ||
      Platform.isIOS ||
      Platform.isWindows ||
      Platform.isLinux;
}
