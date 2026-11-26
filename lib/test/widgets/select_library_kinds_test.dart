import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/home/widgets/select_library.dart';
import 'package:rune/utils/api/request_failure.dart';

final _strings = lookupAppLocalizations(const Locale('en'));

const _records = [
  LibraryRecordSummary(
    recordId: 'local-1',
    alias: 'Local alias',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'l1', path: '/music/Jazz'),
  ),
  LibraryRecordSummary(
    recordId: 'subsonic-1',
    alias: 'Home server',
    kind: LibraryRecordKindSummarySubsonic(
      endpoint: 'https://music.example',
      account: 'me',
    ),
  ),
  LibraryRecordSummary(
    recordId: 'apple-1',
    alias: 'Apple Music',
    kind: LibraryRecordKindSummaryAppleMusic(),
    signedIn: true,
  ),
];

void main() {
  Future<List<String>> pumpPicker(WidgetTester tester) async {
    tester.view.physicalSize = const Size(900, 1000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final opened = <String>[];
    final registry = LibraryRegistryProvider(
      loadRecords: () async => const ListLibraryRecordsResponse(
        records: _records,
        invalidRecords: [],
        lastOpenRecord: null,
      ),
      openRecord: (recordId, callId) async {
        opened.add(recordId);
        throw const RequestFailure(
          'OpenLibraryRequest',
          RequestErrorKind.invalidInput,
          'stop here',
          RequestFailureCauseLibraryOpenFailed(
            kind: OpenLibraryFailureKind.invalidRecord,
          ),
        );
      },
    );
    final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
    final responsive = ResponsiveProvider(sizes, () async => null);
    addTearDown(registry.dispose);
    addTearDown(responsive.dispose);
    addTearDown(sizes.dispose);
    await registry.ready;

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryRegistryProvider>.value(
            value: registry,
          ),
          ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
          ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
        ],
        child: const FluentApp(
          locale: Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(content: SelectLibraryPage()),
        ),
      ),
    );
    await tester.pump();
    return opened;
  }

  testWidgets('the picker lists Subsonic and Apple Music libraries with '
      'their own titles, subtitles and icons', (tester) async {
    await pumpPicker(tester);

    expect(find.text('Jazz'), findsOneWidget);
    expect(find.text('Home server'), findsOneWidget);
    expect(find.text('https://music.example'), findsOneWidget);
    expect(find.text('Apple Music'), findsOneWidget);
    expect(find.text(_strings.appleMusicSubtitle), findsOneWidget);
    // The streaming libraries have the note, the local one the folder.
    expect(find.byIcon(Symbols.music_note), findsNWidgets(2));
    expect(find.byIcon(Symbols.folder), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));
  });

  for (final (title, recordId) in [
    ('Apple Music', 'apple-1'),
    ('Home server', 'subsonic-1'),
  ]) {
    testWidgets('tapping $title opens its record', (tester) async {
      final opened = await pumpPicker(tester);
      await tester.tap(find.text(title));
      await tester.pump();
      expect(opened, [recordId]);
      await tester.pump(const Duration(seconds: 1));
    });
  }
}
