import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/navigation_bar/parent_link.dart';

Widget _app(double textScale) => FluentApp(
  builder: (context, child) => MediaQuery(
    data: MediaQuery.of(
      context,
    ).copyWith(textScaler: TextScaler.linear(textScale)),
    child: child!,
  ),
  home: Align(
    alignment: Alignment.topLeft,
    child: ParentLink(titleFlipKey: 'title', text: 'Library', onPressed: () {}),
  ),
);

Future<double> _width(WidgetTester tester) async {
  await tester.pump();
  return tester.getSize(find.byType(ParentLink)).width;
}

void main() {
  testWidgets('the title width follows the text scale', (tester) async {
    await tester.pumpWidget(_app(1));
    final plain = await _width(tester);

    await tester.pumpWidget(_app(1.3));
    final scaled = await _width(tester);

    // The width is (text width * 5) plus a fixed 12 pixels of padding.
    expect((scaled - 12) / (plain - 12), closeTo(1.3, 0.05));
  });
}
