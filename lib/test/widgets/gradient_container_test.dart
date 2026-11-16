import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/cover_wall_background/gradient_container.dart';
import '../../widgets/cover_wall_background/shader_sampler.dart';

// Counts how often the page around the gradient paints.
class _CountPaints extends SingleChildRenderObjectWidget {
  const _CountPaints({required super.child});

  @override
  _RenderCountPaints createRenderObject(BuildContext context) =>
      _RenderCountPaints();
}

class _RenderCountPaints extends RenderProxyBox {
  int paints = 0;

  @override
  void paint(PaintingContext context, Offset offset) {
    paints++;
    super.paint(context, offset);
  }
}

// The cover wall's gradient inside a layout builder, as the cover wall and
// the lyrics have it, on a page that is shown or hidden.
Widget _page({required bool shown}) {
  return FluentTheme(
    data: FluentThemeData(brightness: Brightness.dark),
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: TickerMode(
        enabled: shown,
        child: Center(
          child: _CountPaints(
            child: SizedBox(
              width: 64,
              height: 48,
              child: LayoutBuilder(
                builder: (context, constraints) => const GradientContainer(
                  gradientParams: GradientParams(),
                  effectParams: EffectParams(),
                  color: Colors.white,
                  color2: Colors.black,
                  child: ColoredBox(color: Colors.white),
                ),
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

// The shader loads from the asset bundle outside the fake clock.
Future<void> _loadShader(WidgetTester tester) async {
  for (
    var i = 0;
    i < 20 && find.byType(ShaderSampler).evaluate().isEmpty;
    i++
  ) {
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
  }
  expect(find.byType(ShaderSampler), findsOneWidget);
}

int _pagePaints(WidgetTester tester) =>
    tester.renderObject<_RenderCountPaints>(find.byType(_CountPaints)).paints;

void main() {
  testWidgets('the gradient moves without painting the page around it', (
    tester,
  ) async {
    await tester.pumpWidget(_page(shown: true));
    await _loadShader(tester);
    await tester.pump(const Duration(milliseconds: 16));
    final pagePaints = _pagePaints(tester);

    for (var i = 0; i < 10; i++) {
      expect(tester.binding.hasScheduledFrame, isTrue);
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(_pagePaints(tester), pagePaints);

    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('the gradient asks for no frames while its page is hidden', (
    tester,
  ) async {
    await tester.pumpWidget(_page(shown: true));
    await _loadShader(tester);
    await tester.pump(const Duration(milliseconds: 16));
    expect(tester.binding.hasScheduledFrame, isTrue);

    await tester.pumpWidget(_page(shown: false));
    await tester.pump(const Duration(milliseconds: 16));
    // Time passes with no frame drawn: nothing may ask for one.
    await tester.binding.delayed(const Duration(milliseconds: 500));
    expect(tester.binding.hasScheduledFrame, isFalse);

    await tester.pumpWidget(_page(shown: true));
    await tester.pump(const Duration(milliseconds: 16));
    expect(tester.binding.hasScheduledFrame, isTrue);

    await tester.pumpWidget(const SizedBox());
  });
}
