import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/tile/fancy_cover.dart';

void main() {
  // CoverArt passes double.infinity when it should fill its parent. A row gives
  // its children unbounded width, so only the height bounds the cover there.
  testWidgets('a text cover that fills a parent of unbounded width paints', (
    tester,
  ) async {
    await tester.pumpWidget(
      const FluentApp(
        home: Row(
          mainAxisAlignment: MainAxisAlignment.start,
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              height: 120,
              child: FancyCover(
                size: double.infinity,
                texts: ('Atlas Album', 'Atlas Artist', '2026'),
              ),
            ),
          ],
        ),
      ),
    );

    expect(tester.takeException(), isNull);
    expect(tester.getSize(find.byType(FancyCover)), const Size(120, 120));
  });
}
