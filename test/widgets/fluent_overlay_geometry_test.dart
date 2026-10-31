import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/navigation_bar/flip_animation_manager.dart';
import 'package:rune/widgets/navigation_bar/flip_text.dart';
import 'package:rune/widgets/navigation_bar/flip_text_animation.dart';

void main() {
  testWidgets('mouse tooltip target is in overlay coordinates', (tester) async {
    await tester.pumpWidget(
      Transform.translate(
        offset: const Offset(40, 20),
        child: Transform.scale(
          scale: 1.5,
          alignment: Alignment.topLeft,
          child: FluentApp(
            home: Align(
              alignment: Alignment.topLeft,
              child: Padding(
                padding: const EdgeInsets.all(100),
                child: Tooltip(
                  message: 'tip',
                  child: const SizedBox(width: 100, height: 50),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    await mouse.addPointer(location: const Offset(200, 180));
    await tester.pump();
    await mouse.moveTo(const Offset(220, 180));
    await tester.pump(const Duration(seconds: 2));
    await tester.pump(const Duration(milliseconds: 500));
    final layout = tester.widget<CustomSingleChildLayout>(
      find.byWidgetPredicate(
        (w) =>
            w is CustomSingleChildLayout &&
            w.delegate.runtimeType.toString() == '_TooltipPositionDelegate',
      ),
    );
    final dynamic delegate = layout.delegate;
    final overlay =
        tester.element(find.byType(Overlay).first).findRenderObject()!
            as RenderBox;
    expect(
      delegate.target as Offset,
      overlay.globalToLocal(const Offset(220, 180)),
    );
  });
  testWidgets('submenu remains open when hovering inside its parent item', (
    tester,
  ) async {
    final controller = FlyoutController();
    addTearDown(controller.dispose);
    await tester.pumpWidget(
      Transform.translate(
        offset: const Offset(40, 20),
        child: Transform.scale(
          scale: 1.5,
          alignment: Alignment.topLeft,
          child: FluentApp(
            home: Center(
              child: FlyoutTarget(
                controller: controller,
                child: Button(
                  child: const Text('open'),
                  onPressed: () {
                    controller.showFlyout(
                      builder: (_) => MenuFlyout(
                        items: [
                          MenuFlyoutSubItem(
                            text: const Text('parent'),
                            showHoverDelay: Duration.zero,
                            items: (_) => [
                              MenuFlyoutItem(
                                text: const Text('child'),
                                onPressed: () {},
                              ),
                            ],
                          ),
                        ],
                      ),
                    );
                  },
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    final center = tester.getCenter(find.text('parent'));
    await mouse.addPointer(location: center);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 700));
    await tester.pumpAndSettle();
    expect(find.text('child'), findsOneWidget);
    await mouse.moveTo(center + const Offset(2, 0));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 400));
    expect(find.text('child'), findsOneWidget);
  });
  testWidgets('inset changes dismiss visible tooltip', (tester) async {
    Widget app(double inset) => FluentApp(
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(viewInsets: EdgeInsets.only(bottom: inset)),
        child: child!,
      ),
      home: const Center(
        child: Tooltip(message: 'tip', child: SizedBox(width: 100, height: 50)),
      ),
    );
    await tester.pumpWidget(app(0));
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    addTearDown(mouse.removePointer);
    final center = tester.getCenter(find.byType(Tooltip));
    await mouse.addPointer(location: center);
    await tester.pump();
    await mouse.moveTo(center + const Offset(2, 0));
    await tester.pump(const Duration(seconds: 2));
    await tester.pump(const Duration(milliseconds: 500));
    expect(
      find.byWidgetPredicate(
        (w) => w.runtimeType.toString() == '_TooltipOverlay',
      ),
      findsOneWidget,
    );
    await tester.pumpWidget(app(100));
    await tester.pump();
    expect(
      find.byWidgetPredicate(
        (w) => w.runtimeType.toString() == '_TooltipOverlay',
      ),
      findsNothing,
    );
  });

  testWidgets('inset changes cancel active flip and restore text', (
    tester,
  ) async {
    final manager = GlobalKey<FlipAnimationManagerState>();
    Widget app(double inset) => FluentApp(
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(viewInsets: EdgeInsets.only(bottom: inset)),
        child: child!,
      ),
      home: FlipAnimationManager(
        key: manager,
        child: const Column(
          children: [
            FlipText(flipKey: 'from', text: 'from'),
            FlipText(flipKey: 'to', text: 'to'),
          ],
        ),
      ),
    );
    await tester.pumpWidget(app(0));
    await tester.pump();
    final result = manager.currentState!.flipAnimation('from', 'to');
    await tester.pump();
    await tester.pump();
    expect(find.byType(FlipTextAnimation), findsOneWidget);
    await tester.pumpWidget(app(100));
    await tester.pump();
    expect(await result, isFalse);
    expect(find.byType(FlipTextAnimation), findsNothing);
    expect(find.text('from'), findsOneWidget);
    expect(find.text('to'), findsOneWidget);
  });
}
