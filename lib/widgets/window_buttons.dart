import 'dart:io';

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';
import 'package:flutter_fullscreen/flutter_fullscreen.dart';

import '../utils/l10n.dart';
import '../utils/router/navigation.dart';
import '../utils/navigation/utils/escape_from_search.dart';
import '../widgets/router/rune_stack.dart';
import '../providers/router_path.dart';

class WindowIconButton extends StatefulWidget {
  final VoidCallback onPressed;
  final Widget? child;

  const WindowIconButton({
    super.key,
    required this.onPressed,
    this.child,
  });

  @override
  State<WindowIconButton> createState() => _WindowIconButtonState();
}

class _WindowIconButtonState extends State<WindowIconButton> {
  bool isHovered = false;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return MouseRegion(
      onEnter: (_) => setState(() => isHovered = true),
      onExit: (_) => setState(() => isHovered = false),
      child: GestureDetector(
        onTap: widget.onPressed,
        child: Container(
          width: 46,
          height: 30,
          decoration: BoxDecoration(
            color: isHovered
                ? theme.resources.textFillColorPrimary.withOpacity(0.08)
                : Colors.transparent,
          ),
          child: widget.child,
        ),
      ),
    );
  }
}

class DragToMoveArea {}

class WindowFrame extends StatefulWidget {
  final Widget child;
  const WindowFrame(this.child, {super.key});

  @override
  State<WindowFrame> createState() => _WindowFrameState();
}

class _WindowFrameState extends State<WindowFrame> with FullScreenListener {
  bool _isMaximized = false;

  _handowWindowEvent(_) {
    if (_isMaximized != appWindow.isMaximized) {
      setState(() {
        _isMaximized = appWindow.isMaximized;
      });
    }
  }

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

    final isSearch = path == '/search';

    return MouseRegion(
      onEnter: _handowWindowEvent,
      onExit: _handowWindowEvent,
      onHover: _handowWindowEvent,
      child: RuneStack(
        children: [
          if (FullScreen.isFullScreen)
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              mainAxisSize: MainAxisSize.max,
              children: [
                Expanded(
                  child: WindowTitleBarBox(
                    child: MoveWindow(),
                  ),
                ),
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
              ],
            ),
          if (!FullScreen.isFullScreen)
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              mainAxisSize: MainAxisSize.max,
              children: [
                Expanded(
                  child: WindowTitleBarBox(
                    child: MoveWindow(),
                  ),
                ),
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
                WindowIconButton(
                  onPressed: () async {
                    appWindow.minimize();
                  },
                ),
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
                  ),
                ),
                WindowIconButton(
                  onPressed: () {
                    appWindow.hide();
                  },
                ),
                appWindow.isMaximized ? SizedBox(width: 2) : SizedBox(width: 7),
              ],
            ),
          widget.child,
        ],
      ),
    );
  }
}

class BackButton extends StatefulWidget {
  const BackButton({
    super.key,
  });

  @override
  State<BackButton> createState() => _BackButtonState();
}

class _BackButtonState extends State<BackButton> {
  @override
  Widget build(BuildContext context) {
    Provider.of<RouterPathProvider>(context);

    return Builder(
      builder: (context) => PaneItem(
        icon: const Center(child: Icon(FluentIcons.back, size: 12.0)),
        title: Text(S.of(context).back),
        body: const SizedBox.shrink(),
        enabled: $canPop(),
      ).build(
        context,
        false,
        () {
          $pop();
          setState(() => {});
        },
        displayMode: PaneDisplayMode.compact,
      ),
    );
  }
}
