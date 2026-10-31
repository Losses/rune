import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'candidate keys reach the input method without ancestor shortcuts',
    (tester) async {
      final controller = TextEditingController();
      final focus = FocusNode();
      final key = GlobalKey<AutoSuggestBoxState<int>>();
      var ancestorKeys = 0;
      var selected = false;
      await tester.pumpWidget(
        FluentApp(
          home: Focus(
            onKeyEvent: (_, event) {
              if (event is KeyDownEvent) ancestorKeys++;
              return KeyEventResult.handled;
            },
            child: AutoSuggestBox<int>(
              key: key,
              controller: controller,
              focusNode: focus,
              items: [
                AutoSuggestBoxItem(
                  value: 1,
                  label: 'nihao',
                  onFocusChange: (value) => selected = value,
                ),
              ],
            ),
          ),
        ),
      );
      focus.requestFocus();
      await tester.pump();
      controller.value = const TextEditingValue(
        text: 'nihao',
        selection: TextSelection.collapsed(offset: 5),
        composing: TextRange(start: 0, end: 5),
      );
      key.currentState!.showOverlay();
      await tester.pump();
      final before = controller.value;
      for (final logicalKey in [
        LogicalKeyboardKey.escape,
        LogicalKeyboardKey.arrowUp,
        LogicalKeyboardKey.arrowDown,
        LogicalKeyboardKey.arrowLeft,
        LogicalKeyboardKey.arrowRight,
        LogicalKeyboardKey.enter,
        LogicalKeyboardKey.numpadEnter,
        LogicalKeyboardKey.space,
        LogicalKeyboardKey.pageUp,
        LogicalKeyboardKey.pageDown,
      ]) {
        final handled = await tester.sendKeyDownEvent(logicalKey);
        await tester.sendKeyUpEvent(logicalKey);
        expect(handled, isFalse, reason: '$logicalKey must reach the platform');
        expect(controller.value, before);
        expect(focus.hasFocus, isTrue);
        expect(key.currentState!.isOverlayVisible, isTrue);
      }
      expect(ancestorKeys, 0);
      controller.value = before.copyWith(composing: TextRange.empty);
      await tester.pump();
      expect(
        await tester.sendKeyDownEvent(LogicalKeyboardKey.arrowDown),
        isTrue,
      );
      await tester.sendKeyUpEvent(LogicalKeyboardKey.arrowDown);
      expect(selected, isTrue);
      expect(ancestorKeys, 0);
      expect(await tester.sendKeyDownEvent(LogicalKeyboardKey.escape), isTrue);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(key.currentState!.isOverlayVisible, isFalse);
      await tester.pumpWidget(const SizedBox());
      controller.dispose();
      focus.dispose();
    },
  );
}
