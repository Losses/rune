import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_viewport.dart';
import 'package:rune/widgets/ax_reveal/ax_reveal.dart';
import 'package:rune/widgets/ax_reveal/utils/reveal_effect_painter.dart';
import 'package:rune/widgets/ax_reveal/widgets/reveal_effect_context.dart';

void main() {
  testWidgets(
    'reprojects a stationary pointer when application scale changes',
    (tester) async {
      final scale = AppScaleProvider(
        read: () async => 100,
        write: (_) async {},
      );
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: FluentApp(
            builder: (context, child) => AppViewport(child: child!),
            home: const RevealEffectContext(
              child: Align(
                alignment: Alignment.topLeft,
                child: SizedBox(
                  width: 200,
                  height: 100,
                  child: AxReveal(child: SizedBox.expand()),
                ),
              ),
            ),
          ),
        ),
      );
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: const Offset(50, 40));
      await mouse.moveTo(const Offset(50, 40));
      await tester.pump();
      await tester.pump();
      RevealEffectPainter painter() => tester
          .widgetList<CustomPaint>(
            find.descendant(
              of: find.byType(AxReveal),
              matching: find.byType(CustomPaint),
            ),
          )
          .map((x) => x.foregroundPainter)
          .whereType<RevealEffectPainter>()
          .single;
      expect(painter().mousePosition, const Offset(50, 40));
      await scale.setPercentage(200);
      await tester.pump();
      await tester.pump();
      await tester.pump();
      expect(painter().mousePosition, const Offset(25, 20));
      await mouse.removePointer();
      await tester.pumpWidget(const SizedBox());
      scale.dispose();
    },
  );
  testWidgets('preserves child state through scale and theme updates', (
    tester,
  ) async {
    final scale = AppScaleProvider(read: () async => 100, write: (_) async {});
    Widget app(Brightness brightness) => ChangeNotifierProvider.value(
      value: scale,
      child: FluentApp(
        theme: FluentThemeData(brightness: brightness),
        builder: (context, child) => AppViewport(child: child!),
        home: const RevealEffectContext(child: AxReveal0(child: _Probe())),
      ),
    );
    await tester.pumpWidget(app(Brightness.light));
    final state = tester.state(find.byType(_Probe));
    await scale.setPercentage(200);
    await tester.pump();
    await tester.pumpWidget(app(Brightness.dark));
    await tester.pump();
    expect(tester.state(find.byType(_Probe)), same(state));
    await tester.pumpWidget(const SizedBox());
    scale.dispose();
  });
}

class _Probe extends StatefulWidget {
  const _Probe();
  @override
  State<_Probe> createState() => _ProbeState();
}

class _ProbeState extends State<_Probe> {
  @override
  Widget build(BuildContext context) => const SizedBox.expand();
}
