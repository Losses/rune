import 'dart:io';

import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:saf_util/saf_util.dart';

import 'ios_file_selector.dart';

Future<String?> getDirPath() async {
  if (Platform.isIOS) {
    return await IosFileSelector.shared.getDirectoryPath();
  }

  // fast_file_picker never passes persistablePermission, so the SAF grant
  // would die with the process. Call saf_util directly on Android to keep
  // access across restarts (essential for TF cards).
  if (Platform.isAndroid) {
    final dir = await SafUtil().pickDirectory(
      writePermission: true,
      persistablePermission: true,
    );

    return dir?.uri;
  }

  final path = await FastFilePicker.pickFolder(writePermission: true);

  if (path == null) {
    return null;
  }

  return path.path ?? path.uri;
}
