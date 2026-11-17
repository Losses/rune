import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/rune_clickable.dart';

Future<void> _pumpClickable(
  WidgetTester tester,
  VoidCallback? onPressed,
) async {
  await tester.pumpWidget(
    FluentApp(
      home: Center(
        child: RuneClickable(
          autofocus: true,
          onPressed: onPressed,
          child: const SizedBox(width: 32, height: 32),
        ),
      ),
    ),
  );
  await tester.pump();
}

Future<void> _mouseClick(WidgetTester tester, int buttons) async {
  final gesture = await tester.startGesture(
    tester.getCenter(find.byType(RuneClickable)),
    kind: PointerDeviceKind.mouse,
    buttons: buttons,
  );
  await gesture.up();
  await tester.pump();
}

void main() {
  for (final entry in {
    'primary': kPrimaryMouseButton,
    'secondary': kSecondaryMouseButton,
    'middle': kMiddleMouseButton,
  }.entries) {
    testWidgets('${entry.key} click only activates for the primary button', (
      tester,
    ) async {
      var activations = 0;
      await _pumpClickable(tester, () => activations++);

      await _mouseClick(tester, entry.value);

      expect(activations, entry.value == kPrimaryMouseButton ? 1 : 0);
    });
  }

  for (final key in [LogicalKeyboardKey.enter, LogicalKeyboardKey.space]) {
    testWidgets('${key.keyLabel} activates the focused control once', (
      tester,
    ) async {
      var activations = 0;
      await _pumpClickable(tester, () => activations++);

      await tester.sendKeyEvent(key);
      await tester.pump();

      expect(activations, 1);
    });
  }

  testWidgets('a disabled control ignores pointer and keyboard activation', (
    tester,
  ) async {
    var activations = 0;
    await _pumpClickable(tester, () => activations++);
    await _pumpClickable(tester, null);

    for (final buttons in [
      kPrimaryMouseButton,
      kSecondaryMouseButton,
      kMiddleMouseButton,
    ]) {
      await _mouseClick(tester, buttons);
    }
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyEvent(LogicalKeyboardKey.space);
    await tester.pump();

    expect(activations, 0);
  });

  testWidgets('dragging away before release does not activate', (tester) async {
    var activations = 0;
    await _pumpClickable(tester, () => activations++);
    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(RuneClickable)),
      kind: PointerDeviceKind.mouse,
    );

    await gesture.moveBy(const Offset(100, 0));
    await gesture.up();
    await tester.pump();

    expect(activations, 0);
  });

  testWidgets('a cancelled pointer does not activate', (tester) async {
    var activations = 0;
    await _pumpClickable(tester, () => activations++);
    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(RuneClickable)),
      kind: PointerDeviceKind.mouse,
    );

    await gesture.cancel();
    await tester.pump();

    expect(activations, 0);
  });
}
