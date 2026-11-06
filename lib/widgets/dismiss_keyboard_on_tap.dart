import 'package:flutter/widgets.dart';

import '../utils/platform.dart';

/// On phones and tablets, unfocuses the focused text field when a tap lands
/// where no other widget takes it. Flutter's mobile text fields keep the
/// keyboard open on a tap elsewhere.
///
/// The tap competes in the gesture arena like any other. A button, a list
/// item or the text field itself is deeper in the tree and wins, so it is
/// never stolen from. Elsewhere this is the only taker.
class DismissKeyboardOnTap extends StatelessWidget {
  const DismissKeyboardOnTap({super.key, required this.child});

  final Widget child;

  static void _unfocusTextField() {
    final focus = FocusManager.instance.primaryFocus;
    final context = focus?.context;
    if (focus == null || context == null) return;
    if (context.findAncestorWidgetOfExactType<EditableText>() == null) return;
    focus.unfocus();
  }

  @override
  Widget build(BuildContext context) {
    if (!isMobile) return child;

    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onTap: _unfocusTextField,
      child: child,
    );
  }
}
