import 'dart:io';

import 'package:file_selector/file_selector.dart';

import 'macos_window_control_button_manager.dart';

// TODO(hexagram): This is a temporary solution, not the best way to handle this callback.
// Once BitsdojoWindow is forked and rewritten, using NSWindowDelagate to listen for windowDidEndSheet and windowWillBeginSheet will be a more general and better solution to handle this.
Future<String?> getDirPath() async {
  final path = await getDirectoryPath();
  if (Platform.isMacOS) {
    MacOSWindowControlButtonManager.setVertical();
  }
  return path;
}
