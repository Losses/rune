import 'package:flutter/services.dart';

class IosFileSelector {
  static final shared = IosFileSelector._();

  IosFileSelector._();

  final platform = MethodChannel('not.ci.rune/ios_file_selector');

  Future<String?> getDirectoryPath() async {
    return await platform.invokeMethod('get_directory_path');
  }
}
