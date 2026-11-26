import 'dart:async';

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
import 'package:rune/screens/settings_library/widgets/progress_button.dart';
import 'package:rune/utils/api/set_sync_settings.dart';
import 'package:rune/widgets/settings/settings_box_toggle.dart';

const _syncNow = 'Sync Now';
const _syncing = 'Syncing';
const _autoShareSync = 'Sync Shared Libraries Automatically';

const _shareLibraryId = 'library-share';

/// A WebDAV library whose data lives in the shared folder.
const _share = LibraryRecordSummary(
  recordId: 'share-1',
  alias: 'Shared folder music',
  shareSync: true,
  kind: LibraryRecordKindSummaryOpenDal(
    libraryId: _shareLibraryId,
    service: OpenDalServiceKind.webDav,
    endpoint: 'https://dav.example/share',
    root: 'music',
  ),
);

/// A WebDAV library whose data lives on this device.
const _deviceOnly = LibraryRecordSummary(
  recordId: 'webdav-1',
  alias: 'Device index music',
  shareSync: false,
  kind: LibraryRecordKindSummaryOpenDal(
    libraryId: 'library-device',
    service: OpenDalServiceKind.webDav,
    endpoint: 'https://dav.example/device',
    root: 'music',
  ),
);

const _local = LibraryRecordSummary(
  recordId: 'local-1',
  alias: 'Local',
  shareSync: false,
  kind: LibraryRecordKindSummaryLocal(
    libraryId: 'library-local',
    path: '/music/Local',
  ),
);

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

GetSyncSettingsResponse _settings({bool autoShareSync = true}) =>
    GetSyncSettingsResponse(
      requestId: 'loaded',
      mediaServer: false,
      broadcast: true,
      acceptRemoteControl: true,
      acceptSync: false,
      autoDeviceSync: false,
      autoShareSync: autoShareSync,
      missingHostFile: MissingHostFilePolicy.keepAsRemote,
    );

/// Every field a change can carry, by name. A null value is a field the save
/// leaves as the hub holds it.
Map<String, Object?> _fields(SyncSettingsChange change) => {
  'mediaServer': change.mediaServer,
  'broadcast': change.broadcast,
  'acceptRemoteControl': change.acceptRemoteControl,
  'acceptSync': change.acceptSync,
  'autoDeviceSync': change.autoDeviceSync,
  'autoShareSync': change.autoShareSync,
  'missingHostFile': change.missingHostFile,
};

/// A library manager whose share syncs end when the test says so.
class _Syncs {
  final requested = <String>[];
  final _pending = <String, Completer<ShareSyncFailure?>>{};
  late final manager = LibraryManagerProvider(
    sendScanRequest: (_) {},
    libraryTaskStates: const Stream.empty(),
    requestFailures: const Stream.empty(),
    syncShareLibrary: (libraryId) {
      requested.add(libraryId);
      return (_pending[libraryId] = Completer<ShareSyncFailure?>()).future;
    },
  );

  void end(String libraryId, [ShareSyncFailure? failure]) =>
      _pending.remove(libraryId)!.complete(failure);

  void fail(String libraryId, Object error) =>
      _pending.remove(libraryId)!.completeError(error);
}

Future<void> _pumpPage(
  WidgetTester tester, {
  required List<LibraryRecordSummary> records,
  LibraryManagerProvider? manager,
  Future<GetSyncSettingsResponse> Function()? loadSyncSettings,
  Future<void> Function(SyncSettingsChange)? saveSyncSettings,
}) async {
  final registry = LibraryRegistryProvider(
    loadRecords: () async =>
        ListLibraryRecordsResponse(records: records, invalidRecords: const []),
  );
  final broadcast = _Broadcast();
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  final libraryManager = manager ?? _Syncs().manager;
  addTearDown(registry.dispose);
  addTearDown(broadcast.dispose);
  if (manager == null) addTearDown(libraryManager.dispose);
  addTearDown(responsive.dispose);
  addTearDown(sizes.dispose);
  await registry.ready;

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
        ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
        ChangeNotifierProvider<LibraryManagerProvider>.value(
          value: libraryManager,
        ),
        ChangeNotifierProvider<ScreenSizeProvider>.value(value: sizes),
        ChangeNotifierProvider<ResponsiveProvider>.value(value: responsive),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: SettingsLibraryPage(
          loadLibraries: () async =>
              const ListLocalLibrariesResponse(libraries: []),
          loadSyncSettings: loadSyncSettings ?? () async => _settings(),
          saveSyncSettings: saveSyncSettings ?? (_) async {},
        ),
      ),
    ),
  );
  // Lets the screen size provider's debounce fire while the page is mounted.
  await tester.pump(const Duration(seconds: 1));
  await tester.pump();
}

Finder _row(LibraryRecordSummary record) =>
    find.ancestor(of: find.text(record.alias), matching: find.byType(ListTile));

Finder _inRow(LibraryRecordSummary record, Finder matching) =>
    find.descendant(of: _row(record), matching: matching);

/// Taps [target] and lets the tapped control finish showing the press, which
/// Fluent controls end on a timer.
Future<void> _tap(WidgetTester tester, Finder target) async {
  await tester.tap(target);
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 100));
}

/// Selects a library's row, which shows its actions.
Future<void> _select(WidgetTester tester, LibraryRecordSummary record) async {
  await tester.ensureVisible(find.text(record.alias));
  await _tap(tester, find.text(record.alias));
}

Finder _switchFinder(String title) => find.descendant(
  of: find.widgetWithText(SettingsBoxToggle, title),
  matching: find.byType(ToggleSwitch),
);

void main() {
  testWidgets('only a library stored in a shared folder can be synced now', (
    tester,
  ) async {
    await _pumpPage(tester, records: const [_local, _deviceOnly, _share]);

    for (final record in [_local, _deviceOnly]) {
      await _select(tester, record);
      expect(
        _inRow(record, find.widgetWithText(Button, 'Switch to')),
        findsOneWidget,
        reason: record.alias,
      );
      expect(find.text(_syncNow), findsNothing, reason: record.alias);
    }

    await _select(tester, _share);
    expect(
      _inRow(_share, find.widgetWithText(Button, _syncNow)),
      findsOneWidget,
    );
    expect(find.text(_syncNow), findsOneWidget);
  });

  testWidgets('the sync button shows progress until the sync has ended', (
    tester,
  ) async {
    final syncs = _Syncs();
    addTearDown(syncs.manager.dispose);
    await _pumpPage(tester, records: const [_share], manager: syncs.manager);
    await _select(tester, _share);
    expect(find.byType(ProgressButton), findsNothing);

    await _tap(tester, find.text(_syncNow));

    expect(syncs.requested, [_shareLibraryId]);
    expect(find.text(_syncNow), findsNothing);
    final progress = _inRow(_share, find.byType(ProgressButton));
    expect(progress, findsOneWidget);
    expect(tester.widget<ProgressButton>(progress).title, _syncing);
    expect(tester.widget<ProgressButton>(progress).progress, isNull);
    final running = find.descendant(
      of: progress,
      matching: find.byType(Button),
    );
    expect(tester.widget<Button>(running).onPressed, isNull);
    await _tap(tester, progress);
    expect(syncs.requested, [_shareLibraryId]);

    syncs.end(_shareLibraryId);
    await tester.pump();
    await tester.pump();

    expect(find.byType(ProgressButton), findsNothing);
    final idle = _inRow(_share, find.widgetWithText(Button, _syncNow));
    expect(idle, findsOneWidget);
    expect(tester.widget<Button>(idle).onPressed, isNotNull);
    expect(find.byType(ContentDialog), findsNothing);
  });

  testWidgets('a running sync is still shown after the page is opened again', (
    tester,
  ) async {
    final syncs = _Syncs();
    addTearDown(syncs.manager.dispose);
    await _pumpPage(tester, records: const [_share], manager: syncs.manager);
    await _select(tester, _share);
    await _tap(tester, find.text(_syncNow));

    await tester.pumpWidget(const SizedBox());
    await _pumpPage(tester, records: const [_share], manager: syncs.manager);
    await _select(tester, _share);

    expect(_inRow(_share, find.byType(ProgressButton)), findsOneWidget);
    expect(find.text(_syncNow), findsNothing);

    syncs.end(_shareLibraryId);
    await tester.pump();
    await tester.pump();

    expect(find.byType(ProgressButton), findsNothing);
    expect(find.text(_syncNow), findsOneWidget);
    expect(syncs.requested, [_shareLibraryId]);
  });

  testWidgets('a sync that fails shows why, once', (tester) async {
    final syncs = _Syncs();
    addTearDown(syncs.manager.dispose);
    await _pumpPage(tester, records: const [_share], manager: syncs.manager);
    await _select(tester, _share);
    await _tap(tester, find.text(_syncNow));

    syncs.end(_shareLibraryId, ShareSyncFailure.storeUnavailable);
    await tester.pumpAndSettle();

    expect(find.byType(ContentDialog), findsOneWidget);
    expect(
      find.descendant(
        of: find.byType(ContentDialog),
        // Nothing retries a requested sync, so the message does not say so.
        matching: find.text('Shared storage is unavailable.'),
      ),
      findsOneWidget,
    );
    expect(find.byType(ProgressButton), findsNothing);
    expect(
      _inRow(_share, find.widgetWithText(Button, _syncNow)),
      findsOneWidget,
    );
  });

  testWidgets('a sync request that is refused shows the request error', (
    tester,
  ) async {
    final syncs = _Syncs();
    addTearDown(syncs.manager.dispose);
    await _pumpPage(tester, records: const [_share], manager: syncs.manager);
    await _select(tester, _share);
    await _tap(tester, find.text(_syncNow));

    syncs.fail(_shareLibraryId, Exception('library is not a share library'));
    await tester.pumpAndSettle();

    expect(find.byType(ContentDialog), findsOneWidget);
    expect(
      find.textContaining('library is not a share library'),
      findsOneWidget,
    );
    expect(find.byType(ProgressButton), findsNothing);
    expect(
      _inRow(_share, find.widgetWithText(Button, _syncNow)),
      findsOneWidget,
    );
  });

  testWidgets('the automatic sync switch needs a shared-folder library', (
    tester,
  ) async {
    var loads = 0;
    await _pumpPage(
      tester,
      records: const [_local, _deviceOnly],
      loadSyncSettings: () async {
        loads++;
        return _settings();
      },
    );

    expect(find.text(_local.alias), findsOneWidget);
    expect(find.text(_autoShareSync), findsNothing);
    expect(loads, 0);

    await tester.pumpWidget(const SizedBox());
    await _pumpPage(
      tester,
      records: const [_local, _share],
      loadSyncSettings: () async => _settings(autoShareSync: false),
    );

    expect(find.text(_autoShareSync), findsOneWidget);
    expect(
      tester.widget<ToggleSwitch>(_switchFinder(_autoShareSync)).checked,
      isFalse,
    );
  });

  testWidgets('the automatic sync switch saves only its own setting', (
    tester,
  ) async {
    var loads = 0;
    final saved = <SyncSettingsChange>[];
    await _pumpPage(
      tester,
      records: const [_share],
      loadSyncSettings: () async {
        loads++;
        return _settings();
      },
      saveSyncSettings: (change) async => saved.add(change),
    );
    expect(
      tester.widget<ToggleSwitch>(_switchFinder(_autoShareSync)).checked,
      isTrue,
    );

    await tester.ensureVisible(_switchFinder(_autoShareSync));
    await tester.tap(_switchFinder(_autoShareSync));
    await tester.pumpAndSettle();

    expect(saved.map(_fields), [
      {
        'mediaServer': null,
        'broadcast': null,
        'acceptRemoteControl': null,
        'acceptSync': null,
        'autoDeviceSync': null,
        'autoShareSync': false,
        'missingHostFile': null,
      },
    ]);
    expect(loads, 1, reason: 'a save loaded the settings');
    expect(
      tester.widget<ToggleSwitch>(_switchFinder(_autoShareSync)).checked,
      isFalse,
    );
  });

  testWidgets(
    'the automatic sync switch is disabled while its save is pending',
    (tester) async {
      final pending = Completer<void>();
      final saved = <SyncSettingsChange>[];
      await _pumpPage(
        tester,
        records: const [_share],
        loadSyncSettings: () async => _settings(),
        saveSyncSettings: (change) {
          saved.add(change);
          return pending.future;
        },
      );

      await tester.ensureVisible(_switchFinder(_autoShareSync));
      await tester.tap(_switchFinder(_autoShareSync));
      await tester.pump();

      final waiting = tester.widget<ToggleSwitch>(
        _switchFinder(_autoShareSync),
      );
      expect(waiting.onChanged, isNull);
      expect(waiting.checked, isTrue);
      await tester.tap(_switchFinder(_autoShareSync), warnIfMissed: false);
      await tester.pump();
      expect(saved, hasLength(1));

      pending.complete();
      await tester.pumpAndSettle();

      final done = tester.widget<ToggleSwitch>(_switchFinder(_autoShareSync));
      expect(done.onChanged, isNotNull);
      expect(done.checked, isFalse);
    },
  );

  testWidgets('a failed save of the automatic sync switch keeps its value', (
    tester,
  ) async {
    await _pumpPage(
      tester,
      records: const [_share],
      saveSyncSettings: (_) async =>
          throw Exception('settings file is read-only'),
    );

    await tester.ensureVisible(_switchFinder(_autoShareSync));
    await tester.tap(_switchFinder(_autoShareSync));
    await tester.pumpAndSettle();

    expect(
      find.descendant(
        of: find.byType(ContentDialog),
        matching: find.text(_autoShareSync),
      ),
      findsOneWidget,
    );
    expect(find.textContaining('settings file is read-only'), findsOneWidget);
    expect(
      tester.widget<ToggleSwitch>(_switchFinder(_autoShareSync)).checked,
      isTrue,
    );
  });
}
