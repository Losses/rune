import 'dart:io';

import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:path_provider/path_provider.dart';

import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/banding_animation/branding_animation.dart';

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

Future<File> startSfxFile =
    AssetHelper.instance.getAudioFileFromAssets('assets/startup_1.ogg');

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return const PageContentFrame(
      top: false,
      bottom: false,
      left: false,
      right: false,
      child: BrandingAnimation(),
    );
  }
}
