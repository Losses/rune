import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/dismiss_keyboard_on_tap.dart';

void main() {
  late FocusNode focus;
  late int pressed;
  late bool focusedWhenPressed;

  setUp(() {
    focus = FocusNode();
    pressed = 0;
    focusedWhenPressed = false;
  });

  tearDown(() {
    focus.dispose();
  });

  Future<void> open(WidgetTester tester) async {
    await tester.pumpWidget(
      FluentApp(
        home: DismissKeyboardOnTap(
          child: Column(
            children: [
              TextBox(focusNode: focus, autofocus: true),
              Button(
                onPressed: () {
                  pressed++;
                  focusedWhenPressed = focus.hasFocus;
                },
                child: const Text('Action'),
              ),
              const Expanded(child: SizedBox.expand()),
            ],
          ),
        ),
      ),
    );
    await tester.pump();
    expect(focus.hasFocus, isTrue);
  }

  final mobile = TargetPlatformVariant({
    TargetPlatform.iOS,
    TargetPlatform.android,
  });

  testWidgets('a tap on empty space unfocuses the text field', (tester) async {
    await open(tester);

    await tester.tapAt(const Offset(400, 500));
    await tester.pump();

    expect(focus.hasFocus, isFalse);
  }, variant: mobile);

  testWidgets('a tap on a button runs it with the field still focused', (
    tester,
  ) async {
    await open(tester);

    await tester.tap(find.text('Action'));
    await tester.pump(const Duration(seconds: 1));

    expect(pressed, 1);
    expect(focusedWhenPressed, isTrue);
    expect(focus.hasFocus, isTrue);
  }, variant: mobile);

  testWidgets('a tap on the text field keeps its focus', (tester) async {
    await open(tester);

    await tester.tap(find.byType(TextBox));
    await tester.pump(const Duration(seconds: 1));

    expect(focus.hasFocus, isTrue);
  }, variant: mobile);

  testWidgets('without a focused text field it takes no part in gestures', (
    tester,
  ) async {
    var downs = 0;
    await tester.pumpWidget(
      FluentApp(
        home: DismissKeyboardOnTap(
          child: Column(
            children: [
              TextBox(focusNode: focus),
              Expanded(
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTapDown: (_) => downs++,
                  child: const SizedBox.expand(),
                ),
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pump();
    GestureDetector own() => tester.widget<GestureDetector>(
      find
          .descendant(
            of: find.byType(DismissKeyboardOnTap),
            matching: find.byType(GestureDetector),
          )
          .first,
    );
    expect(own().onTap, isNull);

    // Alone in its arena, a tap is down at once, without its press timeout.
    final gesture = await tester.startGesture(const Offset(400, 500));
    await tester.pump();
    expect(downs, 1);
    await gesture.up();
    await tester.pump(const Duration(seconds: 1));

    // A text field with the focus brings the keyboard and the handler back.
    // The focus moves after a frame, the handler comes with the next one.
    focus.requestFocus();
    await tester.pump();
    await tester.pump();
    expect(own().onTap, isNotNull);
    focus.unfocus();
    await tester.pump();
    await tester.pump();
    expect(own().onTap, isNull);
  }, variant: mobile);

  testWidgets('desktop adds no tap handler of its own', (tester) async {
    await tester.pumpWidget(
      const FluentApp(home: DismissKeyboardOnTap(child: SizedBox.expand())),
    );

    expect(
      find.descendant(
        of: find.byType(DismissKeyboardOnTap),
        matching: find.byType(GestureDetector),
      ),
      findsNothing,
    );
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));

  testWidgets('a focused widget that is not a text field is left alone', (
    tester,
  ) async {
    final other = FocusNode();
    addTearDown(other.dispose);
    await tester.pumpWidget(
      FluentApp(
        home: DismissKeyboardOnTap(
          child: Focus(
            focusNode: other,
            autofocus: true,
            child: const SizedBox.expand(),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(other.hasPrimaryFocus, isTrue);

    await tester.tapAt(const Offset(400, 300));
    await tester.pump();

    expect(other.hasPrimaryFocus, isTrue);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
