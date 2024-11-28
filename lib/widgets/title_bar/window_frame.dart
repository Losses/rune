import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';

import 'window_frame_for_macos.dart';
import 'window_frame_for_windows.dart';

class WindowFrame extends StatelessWidget {
  final Widget child;
  final String? customRouteName;
  const WindowFrame(this.child, {super.key, this.customRouteName});
  
  @override
  Widget build(BuildContext context) {
    if (Platform.isMacOS) {
      return WindowFrameForMacOS(child, customRouteName: customRouteName);
    } else {
      return WindowFrameForWindows(child);
    }
  }
}
