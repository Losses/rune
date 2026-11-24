import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_viewport.dart';

void main() {
  for (final platform in [TargetPlatform.windows, TargetPlatform.macOS]) {
    for (final percentage in AppScaleProvider.supportedPercentages) {
      testWidgets('$platform right click at $percentage%', (tester) async {
        final scale = AppScaleProvider(
          read: () async => percentage,
          write: (_) async {},
        );
        await scale.initialize();
        final controller = TextEditingController(text: 'Some selected text');
        await tester.pumpWidget(
          ChangeNotifierProvider.value(
            value: scale,
            child: FluentApp(
              builder: (context, child) => Transform.translate(
                offset: const Offset(20, 10),
                child: AppViewport(child: child!),
              ),
              home: Align(
                alignment: Alignment.center,
                child: SizedBox(
                  width: 200,
                  height: 40,
                  child: TextBox(controller: controller),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.byType(TextBox));
        controller.selection = TextSelection(
          baseOffset: 0,
          extentOffset: controller.text.length,
        );
        await tester.pumpAndSettle();
        final point = tester.getCenter(find.byType(TextBox));
        final gesture = await tester.startGesture(
          point,
          buttons: kSecondaryMouseButton,
          kind: PointerDeviceKind.mouse,
        );
        await gesture.up();
        await tester.pumpAndSettle();
        final copy = find.text('Copy');
        expect(copy, findsOneWidget);
        final rect = tester.getRect(copy);
        final toolbar = tester.getRect(
          find.ancestor(of: copy, matching: find.byType(Acrylic)).first,
        );
        expect(toolbar.left, greaterThanOrEqualTo(19));
        expect(toolbar.top, greaterThanOrEqualTo(9));
        expect(toolbar.right, lessThanOrEqualTo(821));
        expect(toolbar.bottom, lessThanOrEqualTo(611));
        expect(
          (toolbar.center - point).distance,
          lessThan(180 * scale.scaleFactor),
        );
        expect(rect.right, greaterThan(0));
        expect(rect.left, lessThan(800));
        expect(
          (rect.center - point).distance,
          lessThan(300 * scale.scaleFactor),
        );
        await tester.pumpWidget(const SizedBox());
        controller.dispose();
        scale.dispose();
      }, variant: TargetPlatformVariant.only(platform));
    }
  }
  testWidgets('multiline selection menu stays in viewport at 200%', (
    tester,
  ) async {
    final scale = AppScaleProvider(read: () async => 200, write: (_) async {});
    await scale.initialize();
    final controller = TextEditingController(text: 'first line\nsecond line');
    await tester.pumpWidget(
      ChangeNotifierProvider.value(
        value: scale,
        child: FluentApp(
          builder: (context, child) => AppViewport(child: child!),
          home: Align(
            alignment: Alignment.center,
            child: SizedBox(
              width: 180,
              height: 80,
              child: TextBox(controller: controller, maxLines: 2),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.byType(TextBox));
    controller.selection = TextSelection(
      baseOffset: 0,
      extentOffset: controller.text.length,
    );
    await tester.pumpAndSettle();
    final editable = tester.state<EditableTextState>(find.byType(EditableText));
    expect(editable.showToolbar(), isTrue);
    await tester.pumpAndSettle();
    final toolbar = tester.getRect(
      find
          .ancestor(of: find.text('Copy'), matching: find.byType(Acrylic))
          .first,
    );
    expect(toolbar.left, greaterThanOrEqualTo(-1));
    expect(toolbar.top, greaterThanOrEqualTo(-1));
    expect(toolbar.right, lessThanOrEqualTo(801));
    expect(toolbar.bottom, lessThanOrEqualTo(601));
    expect(
      (toolbar.center - tester.getCenter(find.byType(TextBox))).distance,
      lessThan(350),
    );
    await tester.pumpWidget(const SizedBox());
    controller.dispose();
    scale.dispose();
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));
}
