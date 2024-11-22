import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/utils/router/navigation.dart';
import 'package:window_manager/window_manager.dart';

import '../utils/l10n.dart';
import '../config/app_title.dart';

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
            const Expanded(
              child: DragToMoveArea(
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
