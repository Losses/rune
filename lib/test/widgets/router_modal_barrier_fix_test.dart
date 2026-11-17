import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/router/router_modal_barrier_fix.dart';
import '../../widgets/router/rune_stack.dart';
import '../../widgets/rune_clickable.dart';

void main() {
  testWidgets(
    'an ordinary page leaves clicks in the playback margin to the control',
    (tester) async {
      var presses = 0;
      await tester.pumpWidget(
        FluentApp(
          home: RuneStack(
            children: [
              Align(
                alignment: Alignment.bottomCenter,
                child: RuneClickable(
                  onPressed: () => presses++,
                  child: const Icon(FluentIcons.pause),
                ),
              ),
              const RouterModalBarrierFix(SizedBox.expand()),
            ],
          ),
        ),
      );

      final button = find.byType(RuneClickable);
      await tester.tap(
        button,
        kind: PointerDeviceKind.mouse,
        buttons: kSecondaryMouseButton,
      );
      expect(presses, 0);
      await tester.tap(button, kind: PointerDeviceKind.mouse);
      expect(presses, 1);
    },
  );

  testWidgets('page margins block hits and a dialog still covers the page', (
    tester,
  ) async {
    var backgroundHits = 0;
    var pageTaps = 0;
    await tester.pumpWidget(
      FluentApp(
        home: Stack(
          children: [
            Listener(
              onPointerDown: (_) => backgroundHits++,
              behavior: HitTestBehavior.opaque,
              child: const SizedBox.expand(),
            ),
            RouterModalBarrierFix(
              Button(
                onPressed: () => pageTaps++,
                child: const Text('Page action'),
              ),
            ),
          ],
        ),
      ),
    );

    final page = find.byType(RouterModalBarrierFix);
    await tester.tapAt(tester.getCenter(page));
    expect(backgroundHits, 0);
    expect(pageTaps, 0);

    final button = find.text('Page action');
    final point = tester.getCenter(button);
    await tester.tapAt(point);
    expect(pageTaps, 1);

    final dialog = showDialog<void>(
      context: tester.element(button),
      barrierDismissible: true,
      builder: (_) => const ContentDialog(content: Text('Dialog')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Dialog'), findsOneWidget);
    await tester.tapAt(point);
    await tester.pumpAndSettle();
    expect(find.text('Dialog'), findsNothing);
    expect(pageTaps, 1);
    expect(backgroundHits, 0);
    await dialog;
  });
}
