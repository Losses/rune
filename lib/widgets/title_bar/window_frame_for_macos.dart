import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/is_cover_art_wall_layout.dart';
import '../../utils/macos_window_control_button_manager.dart';
import '../../providers/full_screen.dart';
import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';

import '../router/rune_stack.dart';

import 'drag_move_window_area.dart';

class WindowFrameForMacOS extends StatefulWidget {
  final Widget child;
  final String? customRouteName;
  const WindowFrameForMacOS(this.child, {super.key, this.customRouteName});

  @override
  State<WindowFrameForMacOS> createState() => _WindowFrameForMacOSState();
}

class _WindowFrameForMacOSState extends State<WindowFrameForMacOS> {
  late FullScreenProvider _fullscreen;
  late ResponsiveProvider _responsiveProvider;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _fullscreen = Provider.of<FullScreenProvider>(context, listen: false);
    _responsiveProvider =
        Provider.of<ResponsiveProvider>(context, listen: false);

    _fullscreen.addListener(updateWindowControlButtons);
    _responsiveProvider.addListener(updateWindowControlButtons);
    $router.addListener(updateWindowControlButtons);
  }

  @override
  dispose() {
    super.dispose();
    _fullscreen.removeListener(updateWindowControlButtons);
    _responsiveProvider.removeListener(updateWindowControlButtons);
    $router.removeListener(updateWindowControlButtons);
  }

  void updateWindowControlButtons() {
    var currentDeviceType = _responsiveProvider.currentDeviceType;
    if (currentDeviceType == DeviceType.band ||
        currentDeviceType == DeviceType.dock) {
      MacOSWindowControlButtonManager.shared.setHide();
      return;
    }

    if ((currentDeviceType == DeviceType.zune ||
            currentDeviceType == DeviceType.phone) &&
        isCoverArtWallLayout($router.path)) {
      MacOSWindowControlButtonManager.shared.setHide();
      return;
    }

    MacOSWindowControlButtonManager.shared.setShow();
    MacOSWindowControlButtonManager.shared.setVertical();
    return;
  }

  @override
  Widget build(BuildContext context) {
    if (!Platform.isMacOS) {
      return widget.child;
    }

    final path =
        widget.customRouteName ?? Provider.of<RouterPathProvider>(context).path;

    Provider.of<ScreenSizeProvider>(context);

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
                activeBreakpoint == DeviceType.dock ||
                path == '/' ||
                path == '/scanning') {
              return DragMoveWindowArea();
            }

            return Column(
              children: [
                SizedBox(
                  height: 40,
                  child: Row(
                    children: [
                      SizedBox(
                        width: 320,
                        child: DragMoveWindowArea(isEnabledDoubleTap: false),
                      ),
                      Expanded(child: DragMoveWindowArea())
                    ],
                  ),
                ),
                SizedBox(
                  height: 40,
                  child: Row(
                    children: [
                      Expanded(
                          child: DragMoveWindowArea(isEnabledDoubleTap: false)),
                    ],
                  ),
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
