import 'package:flutter/services.dart';
import 'package:fluent_ui/fluent_ui.dart';

class DeleteDetectingController extends TextEditingController {
  static const String zeroWidthSpace = '\u200B';

  // Notifier to notify when the text is cleared
  final ValueNotifier<bool> isTextClearedNotifier = ValueNotifier<bool>(false);

  DeleteDetectingController({String? text})
      : super(text: text?.isEmpty ?? true ? zeroWidthSpace : text) {
    super.addListener(() {
      if (super.text.isEmpty) {
        // Notify listeners that the text has been cleared
        isTextClearedNotifier.value = true;
      } else {
        isTextClearedNotifier.value = false;
      }

      if (super.text.isEmpty) {
        super.text = zeroWidthSpace;
      }
    });
  }

  String get clearText {
    // Remove the leading zero-width space character when getting the text
    String currentText = super.text;
    if (currentText.startsWith(zeroWidthSpace)) {
      return currentText.substring(1);
    }
    return currentText;
  }

  // Check if the zero-width space character has been deleted
  bool get isDeletePressedInEmptyField {
    String currentText = super.text;
    return currentText.isEmpty || currentText == zeroWidthSpace;
  }

  static final TextInputFormatter digitsOnly =
      FilteringTextInputFormatter.allow(RegExp(r'[0-9]'));

  @override
  void dispose() {
    super.dispose();
    isTextClearedNotifier.dispose();
  }
}
