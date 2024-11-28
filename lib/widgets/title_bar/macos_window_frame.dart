import 'dart:io';

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import '../../main.dart';
import '../../screens/settings_about/settings_about.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../widgets/title_bar/darg_move_window_area.dart';
import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';

import '../router/rune_stack.dart';

import 'window_icon_button.dart';

class WindowFrameForMacOS extends StatefulWidget {
  final Widget child;
  final String? customRouteName;
  const WindowFrameForMacOS(this.child, {super.key, this.customRouteName});

  @override
  State<WindowFrameForMacOS> createState() => _WindowFrameForMacOSState();
}

class _WindowFrameForMacOSState extends State<WindowFrameForMacOS>
    with FullScreenListener {
  @override
  void initState() {
    super.initState();
    FullScreen.addListener(this);
  }

  @override
  dispose() {
    super.dispose();
    FullScreen.removeListener(this);
  }

  @override
  void onFullScreenChanged(bool enabled, SystemUiMode? systemUiMode) {
    setState(() => {});
  }

  @override
  Widget build(BuildContext context) {
    if (!Platform.isMacOS) {
      return widget.child;
    }

    final path = widget.customRouteName ?? Provider.of<RouterPathProvider>(context).path;
    Provider.of<ScreenSizeProvider>(context);

    final isSearch = path == '/search';
    print(114514);
    print(path);

    return RuneStack(
      alignment: Alignment.topLeft,
      children: [
        DeviceTypeBuilder(
          deviceType: const [
            DeviceType.band,
            DeviceType.dock,
            DeviceType.belt,
            DeviceType.zune,
            DeviceType.tv
          ],
          builder: (context, activeBreakpoint) {
            if (activeBreakpoint == DeviceType.band ||
                activeBreakpoint == DeviceType.dock || path == '/' || path == '/scanning') {
              return DargMoveWindowArea();
            }

            return Column(
              children: [
                SizedBox(
                  height: 40,
                  child: Row(
                    children: [
                      SizedBox(
                        width: 320,
                        child: DargMoveWindowArea(isEnabledDoubleTap: false),
                      ),
                      Expanded(child: DargMoveWindowArea())
                    ],
                  ),
                ),
                SizedBox(
                  height: 40,
                  child: Expanded(child: DargMoveWindowArea(isEnabledDoubleTap: false)),
                ),
              ],
            );
          },
        ),
        widget.child,
      ],
    );
  }
}
