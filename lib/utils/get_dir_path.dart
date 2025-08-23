import 'dart:io';

import 'package:fast_file_picker/fast_file_picker.dart';

import 'ios_file_selector.dart';

Future<String?> getDirPath() async {
  if (Platform.isIOS) {
    return await IosFileSelector.shared.getDirectoryPath();
  }
  final path = await FastFilePicker.pickFolder(writePermission: true);

  if (path == null) {
    return null;
  }

  return path.path ?? path.uri;
}
