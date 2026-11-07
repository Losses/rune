import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../screens/settings_devices/paired_renderers_list.dart';

void main() {
  testWidgets('shows renderer availability and removes then refreshes', (
    tester,
  ) async {
    var loads = 0;
    final removed = <String>[];
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: ScaffoldPage(
          content: PairedRenderersList(
            loadRenderers: (libraryIds) async {
              expect(libraryIds, isEmpty);
              loads++;
              return [
                const RendererCandidate(
                  fingerprint: 'fp-bedroom',
                  alias: 'Bedroom',
                  attached: false,
                  availability: RendererAvailability.noControl,
                ),
                const RendererCandidate(
                  fingerprint: 'fp-kitchen',
                  alias: 'Kitchen',
                  attached: false,
                  availability: RendererAvailability.available,
                ),
              ];
            },
            removeRenderer: (fingerprint) async {
              removed.add(fingerprint);
              return true;
            },
            confirmRemove: (_) async => true,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(loads, 1);
    expect(find.text('Bedroom'), findsOneWidget);
    expect(find.text("Can't control this device"), findsOneWidget);
    expect(find.text('Kitchen'), findsOneWidget);

    await tester.tap(find.widgetWithText(Button, 'Remove').first);
    await tester.pumpAndSettle();

    expect(removed, ['fp-bedroom']);
    expect(loads, 2);
    expect(find.text('Remove'), findsNWidgets(2));
  });

  testWidgets('does not remove a renderer when the user cancels', (
    tester,
  ) async {
    final removed = <String>[];
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: ScaffoldPage(
          content: PairedRenderersList(
            loadRenderers: (_) async => [
              const RendererCandidate(
                fingerprint: 'fp-kitchen',
                alias: 'Kitchen',
                attached: false,
                availability: RendererAvailability.available,
              ),
            ],
            removeRenderer: (fingerprint) async {
              removed.add(fingerprint);
              return true;
            },
            confirmRemove: (_) async => false,
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.widgetWithText(Button, 'Remove'));
    await tester.pumpAndSettle();

    expect(removed, isEmpty);
  });
}
