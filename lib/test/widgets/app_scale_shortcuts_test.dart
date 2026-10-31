import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_scale_shortcuts.dart';
import 'package:rune/widgets/app_viewport.dart';

void main() {
  for (final platform in [TargetPlatform.windows, TargetPlatform.macOS]) {
    testWidgets('reset from focused text input on $platform', (tester) async {
      debugDefaultTargetPlatformOverride = platform;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final saved = <int>[];
      final scale = AppScaleProvider(
        read: () async => 200,
        write: (value) async => saved.add(value),
      );
      await scale.initialize();
      final focus = FocusNode();
      final controller = TextEditingController(text: 'preserve me');
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: FluentApp(
            builder: (c, child) =>
                AppViewport(child: AppScaleShortcuts(child: child!)),
            home: TextBox(controller: controller, focusNode: focus),
          ),
        ),
      );
      await tester.tap(find.byType(TextBox));
      await tester.pump();
      final modifier = platform == TargetPlatform.macOS
          ? LogicalKeyboardKey.metaLeft
          : LogicalKeyboardKey.controlLeft;
      await tester.sendKeyDownEvent(modifier);
      await tester.sendKeyEvent(LogicalKeyboardKey.digit0);
      await tester.sendKeyUpEvent(modifier);
      await tester.pumpAndSettle();
      expect(scale.percentage, 100);
      expect(saved, [100]);
      expect(focus.hasFocus, isTrue);
      expect(controller.text, 'preserve me');
      await tester.pumpWidget(const SizedBox());
      scale.dispose();
      controller.dispose();
      focus.dispose();
      debugDefaultTargetPlatformOverride = null;
    });
  }
}
