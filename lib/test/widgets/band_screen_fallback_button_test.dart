import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/band_screen_fallback_button.dart';

void main() {
  testWidgets('tapping the button calls the callback', (tester) async {
    var pressed = 0;
    await tester.pumpWidget(
      FluentApp(
        home: Center(
          child: SizedBox(
            width: 48,
            height: 48,
            child: BandScreenFallbackButton(
              icon: const Icon(FluentIcons.add),
              onPressed: () => pressed++,
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.byType(BandScreenFallbackButton));
    await tester.pump();

    expect(pressed, 1);
  });
}
