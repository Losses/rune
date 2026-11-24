import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/ax_reveal/ax_reveal.dart';
import 'package:rune/widgets/ax_reveal/utils/reveal_effect_painter.dart';
import 'package:rune/widgets/ax_reveal/utils/reveal_effect_controller.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';

void main() {
  testWidgets('hover outside all reveal regions does not request a frame', (
    tester,
  ) async {
    await tester.pumpWidget(
      const FluentApp(
        home: RevealEffectContext(
          child: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 40,
              height: 40,
              child: AxReveal(child: SizedBox.expand()),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final state = tester.state<RevealEffectContextState>(
      find.byType(RevealEffectContext),
    );
    state.updateMousePosition(const Offset(700, 500));
    await tester.pumpAndSettle();
    expect(tester.binding.hasScheduledFrame, isFalse);
    state.updateMousePosition(const Offset(710, 500));
    expect(
      tester.binding.hasScheduledFrame,
      isFalse,
      reason:
          'No reveal surface changes when the pointer stays outside every light radius.',
    );
  });
  testWidgets('coalesced hover updates paint the newest pointer position', (
    tester,
  ) async {
    await tester.pumpWidget(
      const FluentApp(
        home: RevealEffectContext(
          child: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 100,
              height: 100,
              child: AxReveal(child: SizedBox.expand()),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final state = tester.state<RevealEffectContextState>(
      find.byType(RevealEffectContext),
    );
    tester.binding.scheduleFrame();
    state.updateMousePosition(const Offset(10, 10));
    state.updateMousePosition(const Offset(20, 30));
    await tester.pumpAndSettle();
    final painter = tester
        .widgetList<CustomPaint>(
          find.descendant(
            of: find.byType(AxReveal),
            matching: find.byType(CustomPaint),
          ),
        )
        .map((paint) => paint.foregroundPainter)
        .whereType<RevealEffectPainter>()
        .single;
    expect(painter.mousePosition, const Offset(20, 30));
    expect(tester.binding.hasScheduledFrame, isFalse);
    state.updateMousePosition(null);
    await tester.pumpAndSettle();
    final cleared = tester
        .widgetList<CustomPaint>(
          find.descendant(
            of: find.byType(AxReveal),
            matching: find.byType(CustomPaint),
          ),
        )
        .map((paint) => paint.foregroundPainter)
        .whereType<RevealEffectPainter>()
        .single;
    expect(cleared.mousePosition, isNull);
    await tester.pumpWidget(const SizedBox());
    expect(tester.takeException(), isNull);
  });
  testWidgets('active light motion is coalesced until layout completes', (
    tester,
  ) async {
    await tester.pumpWidget(
      const FluentApp(
        home: RevealEffectContext(
          child: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 100,
              height: 100,
              child: AxReveal(child: SizedBox.expand()),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final state = tester.state<RevealEffectContextState>(
      find.byType(RevealEffectContext),
    );
    state.updateMousePosition(const Offset(10, 10));
    await tester.pumpAndSettle();
    final controller = tester
        .widgetList<ListenableBuilder>(
          find.descendant(
            of: find.byType(AxReveal),
            matching: find.byType(ListenableBuilder),
          ),
        )
        .map((widget) => widget.listenable)
        .whereType<RevealEffectController>()
        .single;
    var notifications = 0;
    controller.addListener(() => notifications++);
    for (var i = 11; i <= 30; i++) {
      state.updateMousePosition(Offset(i.toDouble(), 10));
    }
    expect(notifications, 0);
    await tester.pumpAndSettle();
    expect(notifications, 1);
  });

  testWidgets('pending layout projects an entering pointer into new bounds', (
    tester,
  ) async {
    late StateSetter update;
    var left = 0.0;
    await tester.pumpWidget(
      FluentApp(
        home: RevealEffectContext(
          child: StatefulBuilder(
            builder: (context, setState) {
              update = setState;
              return Stack(
                children: [
                  Positioned(
                    left: left,
                    top: 0,
                    width: 40,
                    height: 40,
                    child: const AxReveal(child: SizedBox.expand()),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final state = tester.state<RevealEffectContextState>(
      find.byType(RevealEffectContext),
    );
    state.updateMousePosition(const Offset(700, 500));
    await tester.pumpAndSettle();
    update(() {
      left = 200;
    });
    state.updateMousePosition(const Offset(210, 10));
    await tester.pumpAndSettle();
    final painter = tester
        .widgetList<CustomPaint>(
          find.descendant(
            of: find.byType(AxReveal),
            matching: find.byType(CustomPaint),
          ),
        )
        .map((p) => p.foregroundPainter)
        .whereType<RevealEffectPainter>()
        .single;
    expect(painter.mousePosition, const Offset(10, 10));
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'disposing before a pending pointer update leaves no callback work',
    (tester) async {
      await tester.pumpWidget(
        const FluentApp(
          home: RevealEffectContext(
            child: SizedBox(
              width: 40,
              height: 40,
              child: AxReveal(child: SizedBox.expand()),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final state = tester.state<RevealEffectContextState>(
        find.byType(RevealEffectContext),
      );
      tester.binding.scheduleFrame();
      state.updateMousePosition(const Offset(10, 10));
      await tester.pumpWidget(const SizedBox());
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      expect(tester.binding.hasScheduledFrame, isFalse);
    },
  );
}
