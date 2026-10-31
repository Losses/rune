import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';

class _PaintCounter extends CustomPainter {
  int paints = 0;

  @override
  void paint(Canvas canvas, Size size) => paints++;

  @override
  bool shouldRepaint(_PaintCounter oldDelegate) => false;
}

void main() {
  for (final scale in [0.75, 1.0, 2.0]) {
    testWidgets('hover at scale $scale does not repaint unchanged menu items', (
      tester,
    ) async {
      final previousStrategy = FocusManager.instance.highlightStrategy;
      FocusManager.instance.highlightStrategy =
          FocusHighlightStrategy.alwaysTraditional;
      addTearDown(
        () => FocusManager.instance.highlightStrategy = previousStrategy,
      );
      final controller = FlyoutController();
      final counter = _PaintCounter();
      await tester.pumpWidget(
        FluentApp(
          builder: (_, child) => Transform.scale(
            scale: scale,
            alignment: Alignment.topLeft,
            child: child,
          ),
          home: FlyoutTarget(
            controller: controller,
            child: const SizedBox.expand(),
          ),
        ),
      );
      controller.showFlyout<void>(
        position: const Offset(100, 100),
        builder: (_) => MenuFlyout(
          items: [
            MenuFlyoutItem(text: const Text('First'), onPressed: () {}),
            MenuFlyoutItem(text: const Text('Second'), onPressed: () {}),
            MenuFlyoutItem(
              text: CustomPaint(
                painter: counter,
                child: const Text('Unchanged'),
              ),
              onPressed: () {},
            ),
          ],
        ),
      );
      await tester.pumpAndSettle();
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: const Offset(10, 10));
      await tester.pumpAndSettle();
      final initialPaints = counter.paints;
      expect(initialPaints, greaterThan(0));
      for (final label in ['First', 'Second', 'First', 'Second']) {
        await mouse.moveTo(tester.getCenter(find.text(label)));
        await tester.pump();
        final hover = tester.widget<HoverButtonInherited>(
          find
              .ancestor(
                of: find.text(label),
                matching: find.byType(HoverButtonInherited),
              )
              .first,
        );
        expect(hover.states.isHovered, isTrue);
      }
      expect(counter.paints, initialPaints);
      await mouse.removePointer();
      await tester.pumpWidget(const SizedBox());
      controller.dispose();
    });
  }
}
