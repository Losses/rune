import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../l10n/app_localizations.dart';
import '../../utils/router/navigation.dart';

void main() {
  testWidgets('a modal dialog rises with the on-screen keyboard', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(400, 800);
    addTearDown(tester.view.reset);

    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Button(
            child: const Text('open'),
            onPressed: () => $showModal<void>(
              context,
              (context, close) => ContentDialog(
                title: const Text('Dialog'),
                content: const SizedBox(height: 40, child: TextBox()),
                actions: [
                  Button(
                    onPressed: () => close(null),
                    child: const Text('Close'),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    final resting = tester.getCenter(find.text('Dialog')).dy;

    tester.view.viewInsets = const FakeViewPadding(bottom: 300);
    await tester.pumpAndSettle();
    final raised = tester.getCenter(find.text('Dialog')).dy;

    // The free area shrinks by the keyboard, so the centred dialog moves up
    // by half of it. A larger move would mean the inset was applied twice.
    // Fluent's transition leaves the route page scaled, which scales the move.
    final scale = tester
        .widget<ScaleTransition>(
          find
              .ancestor(
                of: find.byType(ContentDialog),
                matching: find.byType(ScaleTransition),
              )
              .first,
        )
        .scale
        .value;
    expect(resting - raised, closeTo(150 * scale, 0.5));
    expect(
      tester.getRect(find.text('Close')).bottom,
      lessThanOrEqualTo(800 - 300),
    );

    tester.view.resetViewInsets();
    await tester.pumpAndSettle();
    expect(tester.getCenter(find.text('Dialog')).dy, closeTo(resting, 0.5));

    await tester.tap(find.text('Close'));
    await tester.pumpAndSettle();
    expect(find.text('Dialog'), findsNothing);
  });
}
