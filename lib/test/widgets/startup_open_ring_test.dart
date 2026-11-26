import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/router/startup_open_ring.dart';

void main() {
  Future<void> pumpRing(WidgetTester tester) =>
      tester.pumpWidget(const FluentApp(home: StartupOpenRing()));

  testWidgets('an open that ends within the delay shows no ring', (
    tester,
  ) async {
    await pumpRing(tester);
    await tester.pump(startupOpenRingDelay - const Duration(milliseconds: 1));
    expect(find.byType(ProgressRing), findsNothing);

    // The library opened, so the ring is gone before its delay ended.
    await tester.pumpWidget(const FluentApp(home: SizedBox()));
    await tester.pump(const Duration(seconds: 1));
    expect(find.byType(ProgressRing), findsNothing);
  });

  testWidgets('an open that takes longer shows the ring', (tester) async {
    await pumpRing(tester);
    expect(find.byType(ProgressRing), findsNothing);

    await tester.pump(startupOpenRingDelay);
    expect(find.byType(ProgressRing), findsOneWidget);
  });
}
