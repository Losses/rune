import 'dart:io';

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import '../../main.dart';
import '../../utils/router/navigation.dart';
import '../../utils/navigation/utils/escape_from_search.dart';
import '../../widgets/title_bar/darg_move_window_area.dart';
import '../../providers/router_path.dart';
import '../../providers/responsive_providers.dart';

import '../router/rune_stack.dart';

import 'window_icon_button.dart';

class WindowFrameForWindows extends StatefulWidget {
  final Widget child;
  const WindowFrameForWindows(this.child, {super.key});

  @override
  State<WindowFrameForWindows> createState() => _WindowFrameForWindowsState();
}

class _WindowFrameForWindowsState extends State<WindowFrameForWindows>
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
    if (!Platform.isWindows) {
      return widget.child;
    }

    final path = Provider.of<RouterPathProvider>(context).path;
    Provider.of<ScreenSizeProvider>(context);

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
              return DargMoveWindowArea();
            }

            return SizedBox(
              height: 30,
              child: Row(
                mainAxisAlignment: MainAxisAlignment.end,
                children: [
                  Expanded(
                    child: DargMoveWindowArea(),
                  ),
                  if (FullScreen.isFullScreen)
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
                  if (FullScreen.isFullScreen)
                    WindowIconButton(
                      onPressed: () {
                        FullScreen.setFullScreen(false);
                      },
                      child: Center(
                        child: Icon(
                          FluentIcons.full_screen,
                          size: 12,
                        ),
                      ),
                    ),
                  if (!FullScreen.isFullScreen)
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
                  if (!FullScreen.isFullScreen)
                    WindowIconButton(
                      onPressed: () async {
                        appWindow.minimize();
                      },
                      child: isWindows11
                          ? null
                          : Center(
                              child: Icon(
                                FluentIcons.chrome_minimize,
                                size: 12,
                              ),
                            ),
                    ),
                  if (!FullScreen.isFullScreen)
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
                        child: isWindows11
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
                  if (!FullScreen.isFullScreen)
                    WindowIconButton(
                      onPressed: () {
                        appWindow.hide();
                      },
                      child: isWindows11
                          ? null
                          : Center(
                              child: Icon(
                                FluentIcons.chrome_close,
                                size: 12,
                              ),
                            ),
                    ),
                  if (!FullScreen.isFullScreen)
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
  }
}
