import 'dart:io';

import 'package:flutter/services.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:bitsdojo_window/bitsdojo_window.dart';

import '../utils/l10n.dart';
import '../utils/router/navigation.dart';
import '../config/app_title.dart';
import '../providers/router_path.dart';
import 'window_frame/window_caption_button.dart';

/// This class comes from Window Manager, with modification.
/// The original code is licensed under the MIT license
class WindowCaption extends StatefulWidget {
  const WindowCaption({
    super.key,
    this.title,
    this.backgroundColor,
    this.brightness,
  });

  final Widget? title;
  final Color? backgroundColor;
  final Brightness? brightness;

  @override
  State<WindowCaption> createState() => _WindowCaptionState();
}

class _WindowCaptionState extends State<WindowCaption> {
  @override
  void initState() {
    super.initState();
  }

  @override
  void dispose() {
    super.dispose();
  }

  static const platform = MethodChannel('ci.not.rune/snap');

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: BoxDecoration(
        color: widget.backgroundColor ??
            (widget.brightness == Brightness.dark
                ? const Color(0xff1C1C1C)
                : Colors.transparent),
      ),
      child: Row(
        children: [
          Expanded(
            child: WindowTitleBarBox(
              child: SizedBox(
                height: double.infinity,
                child: Row(
                  children: [
                    Container(
                      padding: const EdgeInsets.only(left: 16),
                      child: DefaultTextStyle(
                        style: TextStyle(
                          color: widget.brightness == Brightness.light
                              ? Colors.black.withOpacity(0.8956)
                              : Colors.white,
                          fontSize: 14,
                        ),
                        child: widget.title ?? Container(),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
          WindowCaptionButton.minimize(
            brightness: widget.brightness,
            onPressed: () async {
              appWindow.minimize();
            },
          ),
          MouseRegion(
            onEnter: (event) async {
              await platform.invokeMethod('maximumButtonEnter');
            },
            onExit: (event) async {
              await platform.invokeMethod('maximumButtonExit');
            },
            child: (appWindow.isMaximized)
                ? WindowCaptionButton.unmaximize(
                    brightness: widget.brightness,
                    onPressed: () {
                      appWindow.restore();
                    },
                  )
                : WindowCaptionButton.maximize(
                    brightness: widget.brightness,
                    onPressed: () {
                      appWindow.maximize();
                    },
                  ),
          ),
          WindowCaptionButton.close(
            brightness: widget.brightness,
            onPressed: () {
              appWindow.hide();
            },
          ),
        ],
      ),
    );
  }
}

class DragToMoveArea {}

class WindowButtons extends StatelessWidget {
  const WindowButtons({super.key});

  @override
  Widget build(BuildContext context) {
    final FluentThemeData theme = FluentTheme.of(context);

    return SizedBox(
      width: 138,
      height: 50,
      child: WindowCaption(
        brightness: theme.brightness,
        backgroundColor: Colors.transparent,
      ),
    );
  }
}

class WindowFrame extends StatelessWidget {
  final Widget child;
  const WindowFrame(this.child, {super.key});

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      if (Platform.isWindows)
        Row(
          mainAxisAlignment: MainAxisAlignment.end,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            BackButton(),
            Expanded(
              child: WindowTitleBarBox(
                child: SizedBox(
                  height: 50,
                  child: Align(
                    alignment: AlignmentDirectional.centerStart,
                    child: Text(appTitle),
                  ),
                ),
              ),
            ),
            const WindowButtons(),
          ],
        ),
      Expanded(child: child),
    ]);
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
