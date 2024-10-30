import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/navigation/back_intent.dart';

class BackSpaceAction extends Action<BackIntent> {
  final TextEditingController controller;

  BackSpaceAction(this.controller);

  void _deletePreviousCharacter() {
    final text = controller.text;
    final cursorPosition = controller.selection.baseOffset;

    if (cursorPosition > 0) {
      final newText = text.substring(0, cursorPosition - 1) +
          text.substring(cursorPosition);
      controller.value = TextEditingValue(
        text: newText,
        selection: TextSelection.collapsed(offset: cursorPosition - 1),
      );
    }
  }

  @override
  void invoke(covariant BackIntent intent) {
    _deletePreviousCharacter();
  }
}
