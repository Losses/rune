import 'dart:io';

import 'package:file_selector/file_selector.dart';

import 'ios_file_selector.dart';

Future<String?> getDirPath() async {
  if (Platform.isIOS) {
    return await IosFileSelector.shared.getDirectoryPath();
  }
  final path = await getDirectoryPath();
  return path;
}

