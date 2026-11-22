import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/utils/dialogs/playlist/show_import_m3u8_success_dialog.dart';

void main() {
  Future<void> openResult(
    WidgetTester tester,
    CreateM3u8PlaylistResponse result,
  ) async {
    await tester.pumpWidget(
      FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showM3u8ImportResultDialog(context, result),
            child: const Text('import'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('import'));
    await tester.pumpAndSettle();
  }

  String messageText(WidgetTester tester) =>
      tester.widget<SelectableText>(find.byType(SelectableText)).data!;

  testWidgets('an ambiguity-only import shows the ambiguous entries', (
    tester,
  ) async {
    await openResult(
      tester,
      const CreateM3u8PlaylistResponse(
        importedCount: 2,
        notFoundPaths: [],
        ambiguousPaths: ['disc-a/song.flac', 'disc-b/song.flac'],
      ),
    );

    expect(find.byType(ContentDialog), findsOneWidget);
    expect(find.text('Partially Successful'), findsOneWidget);
    expect(
      find.textContaining('some entries matched multiple files'),
      findsOneWidget,
    );
    expect(messageText(tester), contains('disc-a/song.flac'));
    expect(messageText(tester), contains('disc-b/song.flac'));
    expect(messageText(tester), isNot(contains('Not found')));
  });

  testWidgets('a mixed import shows ambiguous and unmatched entries', (
    tester,
  ) async {
    await openResult(
      tester,
      const CreateM3u8PlaylistResponse(
        importedCount: 1,
        notFoundPaths: ['missing/song.flac'],
        ambiguousPaths: ['disc-a/song.flac'],
      ),
    );

    expect(find.byType(ContentDialog), findsOneWidget);
    final message = messageText(tester);
    expect(message, contains('Matched multiple files:\ndisc-a/song.flac'));
    expect(message, contains('Not found:\nmissing/song.flac'));
  });

  testWidgets('a fully matched import closes without a warning', (
    tester,
  ) async {
    await openResult(
      tester,
      const CreateM3u8PlaylistResponse(
        importedCount: 3,
        notFoundPaths: [],
        ambiguousPaths: [],
      ),
    );

    expect(find.byType(ContentDialog), findsNothing);
  });
}
