import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/main.dart' show cafeMode, rootNavigatorKey;
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/screens/settings_library/settings_library.dart';
import 'package:rune/utils/router/history.dart';
import 'package:rune/widgets/settings/settings_button.dart';

/// Saved libraries kept on a server, which have no folder on this device:
/// each with its service, alias and endpoint.
const _saved = [
  (
    'Subsonic',
    LibraryRecordSummary(
      shareSync: false,
      recordId: 'subsonic-1',
      alias: 'music.example',
      kind: LibraryRecordKindSummarySubsonic(
        endpoint: 'https://music.example/navidrome',
        account: '',
      ),
    ),
    'https://music.example/navidrome',
  ),
  (
    'WebDAV',
    LibraryRecordSummary(
      shareSync: false,
      recordId: 'webdav-1',
      alias: 'music',
      kind: LibraryRecordKindSummaryOpenDal(
        libraryId: 'library-3',
        service: OpenDalServiceKind.webDav,
        endpoint: 'https://dav.example/remote.php/dav',
        root: 'music',
      ),
    ),
    'https://dav.example/remote.php/dav',
  ),
  (
    'S3',
    LibraryRecordSummary(
      shareSync: false,
      recordId: 's3-1',
      alias: 'tunes',
      kind: LibraryRecordKindSummaryOpenDal(
        libraryId: 'library-4',
        service: OpenDalServiceKind.s3,
        endpoint: 'https://s3.example',
        root: 'tunes',
      ),
    ),
    'https://s3.example',
  ),
];

const _replica = LibraryRecordSummary(
  shareSync: false,
  recordId: 'replica-1',
  alias: 'Shared Music',
  kind: LibraryRecordKindSummaryReplica(
    libraryId: 'library-2',
    ownerFingerprint: 'peer-1',
  ),
);

// A replica keeps its catalog in the app's data, so the local libraries list
// it without a folder path.
const _replicaCatalog = LocalLibrary(
  libraryId: 'library-2',
  alias: 'Shared Music',
  path: '',
  ownerFingerprint: 'peer-1',
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

OpenLibraryResponse _opened(String recordId, int callId) => OpenLibraryResponse(
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  recordId: recordId,
  generation: Uint64.fromBigInt(BigInt.one),
);

/// Shows [page] at `/` on the root navigator, with `/library` standing in for
/// the opened library.
Future<void> _pumpPage(
  WidgetTester tester,
  LibraryRegistryProvider registry,
  Widget page,
) async {
  // The router reads it when a page first navigates.
  cafeMode = false;
  final broadcast = _Broadcast();
  final manager = _LibraryManager();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(registry.dispose);
  addTearDown(broadcast.dispose);
  addTearDown(manager.dispose);
  addTearDown(responsive.dispose);
  addTearDown(sizes.dispose);
  addTearDown($history.reset);
  await registry.ready;

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
      ],
      child: FluentApp(
        navigatorKey: rootNavigatorKey,
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        onGenerateRoute: (settings) => FluentPageRoute<void>(
          settings: settings,
          builder: (context) =>
              settings.name == '/library' ? const Text('Library Home') : page,
        ),
      ),
    ),
  );
  await tester.pump();
}

void main() {
  for (final (service, record, endpoint) in _saved) {
    testWidgets('the home page offers a saved $service library to reopen', (
      tester,
    ) async {
      final opened = <String>[];
      final registry = LibraryRegistryProvider(
        loadRecords: () async =>
            ListLibraryRecordsResponse(records: [record], invalidRecords: []),
        openRecord: (recordId, callId) async {
          opened.add(recordId);
          return _opened(recordId, callId);
        },
      );
      await _pumpPage(tester, registry, const HomePage());

      final row = find.widgetWithText(SettingsButton, record.alias);
      expect(row, findsOneWidget);
      expect(tester.widget<SettingsButton>(row).subtitle, endpoint);

      await tester.tap(row);
      await tester.pump();
      await tester.pump(const Duration(seconds: 1));

      expect(opened, [record.recordId]);
      expect(find.text('Library Home'), findsOneWidget);
    });

    testWidgets(
      'Library settings list a saved $service library to switch to or remove',
      (tester) async {
        final removed = <String>[];
        final registry = LibraryRegistryProvider(
          loadRecords: () async => ListLibraryRecordsResponse(
            records: [_replica, record],
            invalidRecords: [],
          ),
          removeRecord: (recordId) async {
            removed.add(recordId);
            return const RemoveLibraryRecordResponse();
          },
        );
        await _pumpPage(
          tester,
          registry,
          SettingsLibraryPage(
            loadLibraries: () async =>
                const ListLocalLibrariesResponse(libraries: [_replicaCatalog]),
          ),
        );
        await tester.pump();

        expect(find.text(record.alias), findsOneWidget);
        expect(find.text(endpoint), findsOneWidget);
        // The replica's catalog belongs to the replica's row alone.
        expect(find.text('Shared Music'), findsOneWidget);

        final row = find.ancestor(
          of: find.text(record.alias),
          matching: find.byType(ListTile),
        );
        await tester.tap(find.text(record.alias));
        await tester.pump();

        final switchTo = find.descendant(
          of: row,
          matching: find.widgetWithText(Button, 'Switch to'),
        );
        expect(switchTo, findsOneWidget);
        expect(tester.widget<Button>(switchTo).onPressed, isNotNull);
        // None of these libraries is stored in a shared folder.
        expect(
          find.descendant(of: row, matching: find.text('Sync Now')),
          findsNothing,
        );

        await tester.tap(
          find.descendant(of: row, matching: find.text('Remove')),
        );
        await tester.pump();

        expect(removed, [record.recordId]);
        expect(find.text(record.alias), findsNothing);
        await tester.pump(const Duration(seconds: 1));
      },
    );
  }
}
