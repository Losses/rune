import 'package:flutter/widgets.dart';

import '../utils/platform.dart';

/// On phones and tablets, unfocuses the focused text field when a tap lands
/// where no other widget takes it. Flutter's mobile text fields keep the
/// keyboard open on a tap elsewhere.
///
/// The tap competes in the gesture arena like any other. A button, a list
/// item or the text field itself is deeper in the tree and wins, so it is
/// never stolen from. Elsewhere this is the only taker.
///
/// It takes part only while a text field has the focus. Otherwise every
/// gesture would have one more member in its arena: a drag that is alone
/// there would wait for the slop before it starts, and a tap would wait for
/// its press timeout before it shows that it is down.
class DismissKeyboardOnTap extends StatefulWidget {
  const DismissKeyboardOnTap({super.key, required this.child});

  final Widget child;

  @override
  State<DismissKeyboardOnTap> createState() => _DismissKeyboardOnTapState();
}

class _DismissKeyboardOnTapState extends State<DismissKeyboardOnTap> {
  bool _editing = false;

  static bool _textFieldFocused() {
    final context = FocusManager.instance.primaryFocus?.context;
    return context?.findAncestorWidgetOfExactType<EditableText>() != null;
  }

  @override
  void initState() {
    super.initState();
    // The focus manager tells its listeners whenever the primary focus moves.
    FocusManager.instance.addListener(_focusChanged);
    _editing = _textFieldFocused();
  }

  @override
  void dispose() {
    FocusManager.instance.removeListener(_focusChanged);
    super.dispose();
  }

  void _focusChanged() {
    final editing = _textFieldFocused();
    if (editing != _editing) setState(() => _editing = editing);
  }

  void _unfocusTextField() {
    if (_textFieldFocused()) FocusManager.instance.primaryFocus?.unfocus();
  }

  @override
  Widget build(BuildContext context) {
    if (!isMobile) return widget.child;

    // The detector stays without a handler, so the child keeps its place in
    // the tree when a text field gains or loses the focus.
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onTap: _editing ? _unfocusTextField : null,
      child: widget.child,
    );
  }
}
