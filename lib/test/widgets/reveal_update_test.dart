import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/ax_reveal/utils/reveal_config.dart';
import 'package:rune/widgets/ax_reveal/utils/reveal_effect_controller.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';

class _Probe extends StatefulWidget {
  const _Probe({super.key});
  @override
  State<_Probe> createState() => _ProbeState();
}

class _ProbeState extends State<_Probe> {
  RevealEffectController? controller;
  int notifications = 0;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    controller ??= RevealEffectController(context, const RevealConfig())
      ..addListener(() => notifications++);
  }

  @override
  Widget build(BuildContext context) => SizedBox(
    key: controller!.widgetKey,
    width: 100,
    height: 100,
    child: const ColoredBox(color: Colors.transparent),
  );
  @override
  void dispose() {
    controller!.dispose();
    super.dispose();
  }
}

void main() {
  testWidgets('hover and press updates leave unaffected reveal controls idle', (
    tester,
  ) async {
    final near = GlobalKey<_ProbeState>();
    final far = GlobalKey<_ProbeState>();
    await tester.pumpWidget(
      FluentApp(
        home: RevealEffectContext(
          child: Stack(
            children: [
              Positioned(left: 0, top: 0, child: _Probe(key: near)),
              Positioned(left: 600, top: 0, child: _Probe(key: far)),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: const Offset(50, 50));
    await tester.pumpAndSettle();
    near.currentState!.notifications = 0;
    far.currentState!.notifications = 0;
    await mouse.moveTo(const Offset(60, 50));
    await tester.pump();
    await tester.pump();
    await tester.pumpAndSettle();
    expect(near.currentState!.controller!.localPosition, const Offset(60, 50));
    expect(near.currentState!.notifications, 1);
    expect(far.currentState!.notifications, 0);
    expect(tester.binding.transientCallbackCount, 0);

    near.currentState!.notifications = 0;
    tester
        .state<RevealEffectContextState>(find.byType(RevealEffectContext))
        .forceRefresh();
    await tester.pumpAndSettle();
    expect(near.currentState!.notifications, 0);
    expect(far.currentState!.notifications, 0);

    near.currentState!.controller!.mouseDown();
    for (var i = 0; i < 5; i++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(near.currentState!.notifications, greaterThan(0));
    expect(far.currentState!.notifications, 0);
    near.currentState!.controller!.mouseUp();
    await tester.pumpAndSettle();
    expect(near.currentState!.controller!.mouseDownAnimateLogicFrame, 0);
    expect(tester.binding.transientCallbackCount, 0);

    near.currentState!.notifications = 0;
    await mouse.removePointer();
    await tester.pumpAndSettle();
    expect(near.currentState!.controller!.localPosition, isNull);
    expect(near.currentState!.notifications, 1);
    expect(far.currentState!.notifications, 0);
    await tester.pumpWidget(const SizedBox());
  });
}
