import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/settings_library/settings_library.dart';
import 'package:rune/utils/api/request_failure.dart';

final _strings = lookupAppLocalizations(const Locale('en'));

const _alias = 'Streaming';

/// A streaming library whose lyrics come from online, which the user allowed
/// or not.
LibraryRecordSummary _lyricsRecord(bool? enabled) => LibraryRecordSummary(
  shareSync: false,
  recordId: 'streaming-record',
  alias: _alias,
  kind: const LibraryRecordKindSummaryAppleMusic(),
  signedIn: true,
  onlineLyrics: enabled,
);

const _refused = RequestFailure(
  'SetOnlineLyricsRequest',
  RequestErrorKind.invalidInput,
  'The record is not a library whose lyrics come from online',
);

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => null;
}

/// The hub as the settings page sees it: the records it lists, and each
/// choice of looking lyrics up online it was sent, as "record enabled".
class _Hub {
  _Hub(this.record);

  final LibraryRecordSummary record;
  final sent = <String>[];
  Object? failure;

  late final registry = LibraryRegistryProvider(
    loadRecords: () async =>
        ListLibraryRecordsResponse(records: [record], invalidRecords: const []),
    setOnlineLyrics: (recordId, enabled) async {
      sent.add('$recordId $enabled');
      if (failure case final failure?) throw failure;
    },
    recordsChanged: const Stream.empty(),
  );
}

Future<_Hub> _pumpSettings(
  WidgetTester tester,
  LibraryRecordSummary record,
) async {
  final hub = _Hub(record);
  final broadcast = _Broadcast();
  final manager = _LibraryManager();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(hub.registry.dispose);
  addTearDown(broadcast.dispose);
  addTearDown(manager.dispose);
  addTearDown(responsive.dispose);
  addTearDown(sizes.dispose);
  await hub.registry.ready;
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(
          value: hub.registry,
        ),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: SettingsLibraryPage(
          loadLibraries: () async =>
              const ListLocalLibrariesResponse(libraries: []),
        ),
      ),
    ),
  );
  await tester.pump();
  await tester.tap(find.text(_alias));
  await tester.pump();
  return hub;
}

Finder _row() =>
    find.ancestor(of: find.text(_alias), matching: find.byType(ListTile));

List<String?> _rowButtons(WidgetTester tester) => tester
    .widgetList<Button>(
      find.descendant(of: _row(), matching: find.byType(Button)),
    )
    .map((button) => (button.child as Text).data)
    .toList();

/// Opens the Lyrics dialog of the row.
Future<void> _openLyrics(WidgetTester tester) async {
  await tester.tap(
    find.descendant(of: _row(), matching: find.text(_strings.lyrics)),
  );
  await tester.pumpAndSettle();
}

bool _switchChecked(WidgetTester tester) =>
    tester.widget<ToggleSwitch>(find.byType(ToggleSwitch)).checked;

void main() {
  testWidgets(
    'a library that says whether it looks lyrics up gets Lyrics before Remove',
    (tester) async {
      await _pumpSettings(tester, _lyricsRecord(false));
      expect(_rowButtons(tester), [
        _strings.switchTo,
        _strings.lyrics,
        _strings.removeLibrary,
      ]);
      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets('a library that says nothing about lyrics gets no Lyrics', (
    tester,
  ) async {
    await _pumpSettings(tester, _lyricsRecord(null));
    expect(_rowButtons(tester), [_strings.switchTo, _strings.removeLibrary]);
    await tester.pump(const Duration(seconds: 1));
  });

  for (final enabled in [false, true]) {
    testWidgets(
      'Lyrics says what is sent and shows what the library does now ($enabled)',
      (tester) async {
        final hub = await _pumpSettings(tester, _lyricsRecord(enabled));
        await _openLyrics(tester);

        final dialog = find.byType(ContentDialog);
        expect(
          find.descendant(of: dialog, matching: find.text(_strings.lyrics)),
          findsOneWidget,
        );
        expect(find.text(_strings.onlineLyricsNotice), findsOneWidget);
        expect(find.text(_strings.lookUpLyricsOnLrclib), findsOneWidget);
        expect(_switchChecked(tester), enabled);
        // The label is on the left and the switch at the right edge.
        expect(
          tester.widget<ToggleSwitch>(find.byType(ToggleSwitch)).leadingContent,
          isTrue,
        );
        // Nothing is sent by looking.
        expect(hub.sent, isEmpty);

        await tester.tap(
          find.descendant(of: dialog, matching: find.text(_strings.close)),
        );
        await tester.pumpAndSettle();
        expect(find.byType(ContentDialog), findsNothing);
        expect(hub.sent, isEmpty);
        await tester.pump(const Duration(seconds: 1));
      },
    );
  }

  testWidgets('the switch sends the choice for the record', (tester) async {
    final hub = await _pumpSettings(tester, _lyricsRecord(false));
    await _openLyrics(tester);

    await tester.tap(find.byType(ToggleSwitch));
    await tester.pumpAndSettle();
    expect(hub.sent, ['streaming-record true']);
    expect(_switchChecked(tester), isTrue);

    await tester.tap(find.byType(ToggleSwitch));
    await tester.pumpAndSettle();
    expect(hub.sent, ['streaming-record true', 'streaming-record false']);
    expect(_switchChecked(tester), isFalse);
    // No failure was shown over the dialog.
    expect(find.byType(ContentDialog), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('a choice the hub refuses turns the switch back and says so', (
    tester,
  ) async {
    final hub = await _pumpSettings(tester, _lyricsRecord(false));
    hub.failure = _refused;
    await _openLyrics(tester);

    await tester.tap(find.byType(ToggleSwitch));
    await tester.pumpAndSettle();
    expect(hub.sent, ['streaming-record true']);
    expect(_switchChecked(tester), isFalse);
    // The failure is shown as this page shows its other failures.
    expect(find.byType(ContentDialog), findsNWidgets(2));
    expect(find.text(_strings.unknownError), findsOneWidget);
    await tester.tap(find.byType(FilledButton).last);
    await tester.pumpAndSettle();
    expect(find.byType(ContentDialog), findsOneWidget);
    expect(_switchChecked(tester), isFalse);
    await tester.pump(const Duration(seconds: 1));
  });
}
