import 'dart:io';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../../main.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../providers/router_path.dart';
import '../../providers/full_screen.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/linux_custom_window_controls.dart';

import '../router/rune_stack.dart';

import 'window_icon_button.dart';
import 'drag_move_window_area.dart';

class WindowFrameForWindows extends StatefulWidget {
  final Widget child;
  const WindowFrameForWindows(this.child, {super.key});

  @override
  State<WindowFrameForWindows> createState() => _WindowFrameForWindowsState();
}

class _WindowFrameForWindowsState extends State<WindowFrameForWindows> {
  @override
  Widget build(BuildContext context) {
    // Use Consumer to listen to Linux custom window controls changes
    return Consumer<LinuxCustomWindowControlsProvider>(
      builder: (context, linuxControlsProvider, child) {
        // Show custom frame for Windows or Linux with custom controls enabled
        final shouldShowCustomFrame = Platform.isWindows ||
            (Platform.isLinux && linuxControlsProvider.enabled);

        if (!shouldShowCustomFrame) {
          return widget.child;
        }

        final path = Provider.of<RouterPathProvider>(context).path;
        Provider.of<ScreenSizeProvider>(context);

        final fullScreen = Provider.of<FullScreenProvider>(context);

        final isSearch = path == '/search';

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
                    activeBreakpoint == DeviceType.dock) {
                  return DragMoveWindowArea();
                }

                return SizedBox(
                  height: 30,
                  child: Row(
                    mainAxisAlignment: MainAxisAlignment.end,
                    children: [
                      Expanded(
                        child: DragMoveWindowArea(),
                      ),
                      if (fullScreen.isFullScreen)
                        WindowIconButton(
                          onPressed: () {
                            if (isSearch) {
                              escapeFromSearch();
                            } else {
                              $push('/search');
                            }
                          },
                          child: Center(
                            child: Icon(
                              FluentIcons.search,
                              size: 12,
                            ),
                          ),
                        ),
                      if (fullScreen.isFullScreen)
                        WindowIconButton(
                          onPressed: () {
                            fullScreen.setFullScreen(false);
                          },
                          child: Center(
                            child: Icon(
                              FluentIcons.full_screen,
                              size: 12,
                            ),
                          ),
                        ),
                      if (!fullScreen.isFullScreen)
                        activeBreakpoint == DeviceType.zune ||
                                activeBreakpoint == DeviceType.belt
                            ? Container()
                            : WindowIconButton(
                                onPressed: () {
                                  if (isSearch) {
                                    escapeFromSearch();
                                  } else {
                                    $push('/search');
                                  }
                                },
                                child: Center(
                                  child: Icon(
                                    FluentIcons.search,
                                    size: 12,
                                  ),
                                ),
                              ),
                      if (!fullScreen.isFullScreen)
                        WindowIconButton(
                          onPressed: () async {
                            appWindow.minimize();
                          },
                          child: (Platform.isWindows && isWindows11)
                              ? null
                              : Center(
                                  child: Icon(
                                    FluentIcons.chrome_minimize,
                                    size: 12,
                                  ),
                                ),
                        ),
                      if (!fullScreen.isFullScreen)
                        MouseRegion(
                          onEnter: (event) async {
                            // await platform.invokeMethod('maximumButtonEnter');
                          },
                          onExit: (event) async {
                            // await platform.invokeMethod('maximumButtonExit');
                          },
                          child: WindowIconButton(
                            onPressed: () {
                              setState(() {
                                appWindow.maximizeOrRestore();
                              });
                            },
                            child: (Platform.isWindows && isWindows11)
                                ? null
                                : Center(
                                    child: Icon(
                                      appWindow.isMaximized
                                          ? FluentIcons.chrome_restore
                                          : FluentIcons.square_shape,
                                      size: 12,
                                    ),
                                  ),
                          ),
                        ),
                      if (!fullScreen.isFullScreen)
                        WindowIconButton(
                          onPressed: () {
                            appWindow.close();
                          },
                          child: (Platform.isWindows && isWindows11)
                              ? null
                              : Center(
                                  child: Icon(
                                    FluentIcons.chrome_close,
                                    size: 12,
                                  ),
                                ),
                        ),
                      if (!fullScreen.isFullScreen)
                        appWindow.isMaximized
                            ? SizedBox(width: 2)
                            : SizedBox(width: 7),
                    ],
                  ),
                );
              },
            ),
            widget.child,
          ],
        );
      },
    );
  }
}
