import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/ax_pressure.dart';

void main() {
  testWidgets('pan zoom uses the local origin and displacement', (
    tester,
  ) async {
    await tester.pumpWidget(
      const MaterialApp(
        home: Align(
          alignment: Alignment.topLeft,
          child: Padding(
            padding: EdgeInsets.only(left: 100, top: 80),
            child: SizedBox(
              width: 100,
              height: 100,
              child: AxPressure(child: SizedBox.expand()),
            ),
          ),
        ),
      ),
    );
    final listener = tester.widget<Listener>(
      find
          .descendant(
            of: find.byType(AxPressure),
            matching: find.byType(Listener),
          )
          .first,
    );
    listener.onPointerPanZoomUpdate!(
      const PointerPanZoomUpdateEvent(
        position: Offset(50, 50),
        pan: Offset(10, 0),
      ),
    );
    await tester.pump();
    final container = tester.widget<Container>(
      find
          .descendant(
            of: find.byType(AxPressure),
            matching: find.byType(Container),
          )
          .first,
    );
    // A point just right of center tilts around Y; global widget origin is irrelevant.
    expect(container.transform!.entry(0, 2).abs(), lessThan(0.1));
    expect(container.transform!.entry(0, 2).abs(), greaterThan(0.001));
  });
  testWidgets('changing viewport metrics cancels an active tilt', (
    tester,
  ) async {
    Widget build(double ratio) => MediaQuery(
      data: MediaQueryData(size: const Size(800, 600), devicePixelRatio: ratio),
      child: const Directionality(
        textDirection: TextDirection.ltr,
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(
            width: 100,
            height: 100,
            child: AxPressure(child: SizedBox.expand()),
          ),
        ),
      ),
    );
    await tester.pumpWidget(build(1));
    Listener listener() => tester.widget<Listener>(
      find
          .descendant(
            of: find.byType(AxPressure),
            matching: find.byType(Listener),
          )
          .first,
    );
    Matrix4 transform() => tester
        .widget<Container>(
          find
              .descendant(
                of: find.byType(AxPressure),
                matching: find.byType(Container),
              )
              .first,
        )
        .transform!;
    listener().onPointerDown!(const PointerDownEvent(position: Offset(80, 50)));
    await tester.pump();
    expect(transform().entry(0, 2).abs(), greaterThan(0.001));
    await tester.pumpWidget(build(2));
    expect(transform().entry(0, 2), 0);
    listener().onPointerMove!(const PointerMoveEvent(position: Offset(90, 50)));
    await tester.pump();
    expect(transform().entry(0, 2), 0);
    listener().onPointerUp!(const PointerUpEvent(position: Offset(90, 50)));
    listener().onPointerDown!(const PointerDownEvent(position: Offset(80, 50)));
    await tester.pump();
    expect(transform().entry(0, 2).abs(), greaterThan(0.001));
  });
}
