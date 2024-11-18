import 'dart:io';

import 'package:flutter/services.dart';
import 'package:path_provider/path_provider.dart';

class AssetHelper {
  AssetHelper._privateConstructor();

  static final AssetHelper _instance = AssetHelper._privateConstructor();

  static AssetHelper get instance => _instance;

  Future<File> getAudioFileFromAssets(String asset) async {
    final byteData = await rootBundle.load(asset);
    final tempFile = File(
        "${(await getTemporaryDirectory()).path}/${asset.split('/').last}");
    final file = await tempFile.writeAsBytes(
      byteData.buffer
          .asUint8List(byteData.offsetInBytes, byteData.lengthInBytes),
    );
    return file;
  }
}
