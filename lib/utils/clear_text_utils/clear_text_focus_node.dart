import 'package:fluent_ui/fluent_ui.dart';

import './clear_text_controller.dart';

class DeleteDetectingFocusNode extends FocusNode {
  final DeleteDetectingController controller;
  final bool clearIfunfocus;

  DeleteDetectingFocusNode(this.controller, this.clearIfunfocus) {
    addListener(_handleFocusChange);
  }

  void _handleFocusChange() {
    if (hasFocus) {
      if (controller.text.isEmpty) {
        controller.text = '\u200B';
      }
    } else {
      if (controller.text == '\u200B' && clearIfunfocus) {
        controller.clear();
      }
    }
  }

  @override
  void dispose() {
    removeListener(_handleFocusChange);
    super.dispose();
  }
}
