import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/cover_wall_background/shader_sampler.dart';

// Counts how often the page around the sampler paints.
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

class _Repaint extends ChangeNotifier {
  void fire() => notifyListeners();
}

class _Child extends StatefulWidget {
  const _Child({super.key});

  @override
  State<_Child> createState() => _ChildState();
}

class _ChildState extends State<_Child> {
  Color color = Colors.white;

  void change() => setState(() => color = Colors.black);

  @override
  Widget build(BuildContext context) => ColoredBox(color: color);
}

class _Harness {
  final repaint = _Repaint();
  final images = <ui.Image>[];
  final childKey = GlobalKey<_ChildState>();

  Widget build({bool layeredChild = false}) {
    final Widget child = _Child(key: childKey);
    return Directionality(
      textDirection: TextDirection.ltr,
      child: Center(
        child: _CountPaints(
          child: SizedBox(
            width: 40,
            height: 20,
            child: LayoutBuilder(
              builder: (context, constraints) => ShaderSampler(
                (image, size, canvas) {
                  images.add(image);
                  canvas.drawImage(image, Offset.zero, Paint());
                },
                repaint: repaint,
                child: layeredChild ? RepaintBoundary(child: child) : child,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

int _pagePaints(WidgetTester tester) =>
    tester.renderObject<_RenderCountPaints>(find.byType(_CountPaints)).paints;

void main() {
  testWidgets('a repaint draws the sampler again and not the page', (
    tester,
  ) async {
    final harness = _Harness();
    addTearDown(harness.repaint.dispose);
    await tester.pumpWidget(harness.build());
    final drawings = harness.images.length;
    final pagePaints = _pagePaints(tester);

    harness.repaint.fire();
    expect(tester.binding.hasScheduledFrame, isTrue);
    await tester.pump();

    expect(harness.images.length, drawings + 1);
    expect(_pagePaints(tester), pagePaints);
  });

  testWidgets('the snapshot is kept until the child paints again', (
    tester,
  ) async {
    final harness = _Harness();
    addTearDown(harness.repaint.dispose);
    await tester.pumpWidget(harness.build());

    harness.repaint.fire();
    await tester.pump();
    harness.repaint.fire();
    await tester.pump();
    final last = harness.images.length - 1;
    expect(identical(harness.images[last], harness.images[last - 1]), isTrue);

    harness.childKey.currentState!.change();
    await tester.pump();
    expect(identical(harness.images.last, harness.images[last]), isFalse);
  });

  testWidgets('a child with a layer of its own is taken for each drawing', (
    tester,
  ) async {
    final harness = _Harness();
    addTearDown(harness.repaint.dispose);
    await tester.pumpWidget(harness.build(layeredChild: true));

    harness.repaint.fire();
    await tester.pump();
    harness.repaint.fire();
    await tester.pump();
    final last = harness.images.length - 1;
    expect(identical(harness.images[last], harness.images[last - 1]), isFalse);
  });
}
