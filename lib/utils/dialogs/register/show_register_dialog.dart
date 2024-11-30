import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';

import '../../l10n.dart';
import '../../router/navigation.dart';

class RegisterDialog extends StatefulWidget {
  final void Function(void) $close;

  const RegisterDialog({super.key, required this.$close});

  @override
  State<RegisterDialog> createState() => _RegisterDialogState();
}

class _RegisterDialogState extends State<RegisterDialog> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(S.of(context).evaluationMode),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              S.of(context).evaluationModeContent1,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 4),
            Text(
              S.of(context).evaluationModeContent2,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 4),
            Text(
              S.of(context).evaluationModeContent4,
              style: TextStyle(height: 1.4),
            ),
            SizedBox(height: 16),
            Button(
              onPressed: () => {},
              child: Row(
                children: [
                  SvgPicture.asset(
                    width: 16,
                    'assets/bandcamp.svg',
                    colorFilter: ColorFilter.mode(
                      theme.inactiveColor,
                      BlendMode.srcIn,
                    ),
                  ),
                  SizedBox(width: 8),
                  Text("Bandcamp"),
                ],
              ),
            ),
            SizedBox(height: 4),
            Button(
              onPressed: () => {},
              child: Row(
                children: [
                  SvgPicture.asset(
                    width: 16,
                    'assets/itch.svg',
                    colorFilter: ColorFilter.mode(
                      theme.inactiveColor,
                      BlendMode.srcIn,
                    ),
                  ),
                  SizedBox(width: 8),
                  Text("itch.io"),
                ],
              ),
            ),
          ],
        ),
        actions: [
          ResponsiveDialogActions(
            FilledButton(
              onPressed: () async {
                widget.$close(null);
              },
              child: Text(S.of(context).register),
            ),
            Button(
              onPressed: () {
                widget.$close(null);
              },
              child: Text(S.of(context).close),
            ),
          ),
        ],
      ),
    );
  }
}

void showRegisterDialog(BuildContext context) {
  $showModal<void>(
    context,
    (context, $close) => RegisterDialog(
      $close: $close,
    ),
    barrierDismissible: false,
    dismissWithEsc: true,
  );
}
