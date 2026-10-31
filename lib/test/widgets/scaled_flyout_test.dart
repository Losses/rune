import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_viewport.dart';

void main() {
  for (final percentage in AppScaleProvider.supportedPercentages) {
    testWidgets('flyout hover and scale invalidation at $percentage%', (
      tester,
    ) async {
      final scale = AppScaleProvider(
        read: () async => percentage,
        write: (_) async {},
      );
      await scale.initialize();
      final flyout = FlyoutController();
      final home = Stack(
        children: [
          Positioned(
            left: 70,
            top: 40,
            child: FlyoutTarget(
              controller: flyout,
              child: SizedBox(
                width: 100,
                height: 40,
                child: Button(
                  child: const Text('open'),
                  onPressed: () => flyout.showFlyout<void>(
                    dismissOnPointerMoveAway: true,
                    placementMode: FlyoutPlacementMode.bottomLeft,
                    builder: (context) => const FlyoutContent(
                      child: SizedBox(
                        width: 100,
                        height: 60,
                        child: Text('popup'),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      );
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: Padding(
            padding: const EdgeInsets.only(left: 30, top: 20),
            child: FluentApp(
              builder: (context, child) => AppViewport(child: child!),
              home: home,
            ),
          ),
        ),
      );
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      final inside = tester.getCenter(find.byType(FlyoutContent));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: inside);
      await tester.pump();
      await mouse.moveTo(inside + const Offset(1, 0));
      await tester.pumpAndSettle();
      expect(find.text('popup'), findsOneWidget);
      await mouse.moveTo(tester.getCenter(find.text('open')));
      await tester.pumpAndSettle();
      expect(find.text('popup'), findsOneWidget);
      await mouse.moveTo(const Offset(40, 30));
      await tester.pumpAndSettle();
      expect(find.text('popup'), findsNothing);
      await mouse.removePointer();
      await tester.tap(find.text('open'));
      await tester.pumpAndSettle();
      await scale.setPercentage(percentage == 100 ? 125 : 100);
      await tester.pumpAndSettle();
      expect(find.text('popup'), findsNothing);
      expect(flyout.isOpen, isFalse);
      expect(find.text('open'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
      flyout.dispose();
      scale.dispose();
    });
  }
}
