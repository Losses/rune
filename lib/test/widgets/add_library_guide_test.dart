import 'dart:async';
import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/main.dart' show cafeMode, rootNavigatorKey;
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/providers/router_path.dart';
import 'package:rune/utils/add_library_flows.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/utils/dialogs/add_library/add_library_guide.dart';
import 'package:rune/utils/router/history.dart';
import 'package:rune/utils/hosted_player/apple_music_host.dart';
import 'package:rune/utils/hosted_player/hosted_player.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

LibraryRecordSummary _record() => const LibraryRecordSummary(
  shareSync: false,
  recordId: 'record-1',
  alias: 'music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

LibraryRegistryProvider _emptyRegistry({
  Future<void> Function(String path)? saveBookmark,
}) => LibraryRegistryProvider(
  loadRecords: () async => ListLibraryRecordsResponse(
    records: const [],
    invalidRecords: const [],
    lastOpenRecord: null,
  ),
  saveBookmark: saveBookmark,
);

LibraryManagerProvider _testManager() =>
    LibraryManagerProvider(sendScanRequest: (_) {});

OpenLibraryResponse _opened(String recordId, int callId) => OpenLibraryResponse(
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  recordId: recordId,
  generation: Uint64.fromBigInt(BigInt.from(1)),
);

Widget _app({
  required LibraryRegistryProvider registry,
  required LibraryManagerProvider manager,
  required Widget home,
  _Broadcast? broadcast,
}) {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);
  final devices = broadcast ?? _Broadcast();
  addTearDown(devices.dispose);
  return MultiProvider(
    providers: [
      ChangeNotifierProvider.value(value: sizes),
      ChangeNotifierProvider.value(value: responsive),
      ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
      ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
      ChangeNotifierProvider<BroadcastProvider>.value(value: devices),
    ],
    child: FluentApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: home,
    ),
  );
}

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  _Broadcast({List<ClientSummary> pairedServers = const []})
    : _pairedServers = pairedServers;

  List<ClientSummary> _pairedServers;

  /// The list the next [fetchPairedServers] applies, as the hub would report it.
  List<ClientSummary>? nextPairedServers;
  int pairedServerFetches = 0;

  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  List<ClientSummary> get pairedServers => _pairedServers;

  @override
  bool get isLoadingPairedServers => false;

  @override
  void fetchPairedServers() {
    pairedServerFetches++;
    final next = nextPairedServers;
    if (next == null) return;
    nextPairedServers = null;
    _pairedServers = next;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

ClientSummary _server(String alias, ClientStatus status) => ClientSummary(
  alias: alias,
  fingerprint: alias.toLowerCase(),
  deviceModel: 'Desktop',
  status: status,
);

/// Opens the real guide and confirms Rune Device, with no injected devices.
Future<void> _openDeviceStep(
  WidgetTester tester,
  _Broadcast broadcast, {
  Future<void> Function(BuildContext)? searchNeighbors,
}) async {
  final registry = _emptyRegistry();
  final manager = _testManager();
  addTearDown(registry.dispose);
  addTearDown(manager.dispose);
  await tester.pumpWidget(
    _app(
      registry: registry,
      manager: manager,
      broadcast: broadcast,
      home: Builder(
        builder: (context) => Button(
          onPressed: () => showAddLibraryGuide(
            context,
            dependencies: AddLibraryGuideDependencies(
              searchNeighbors: searchNeighbors ?? (_) async {},
              addNeighborManually: (_) async => null,
            ),
          ),
          child: const Text('Open guide'),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open guide'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Rune Device'));
  await tester.pumpAndSettle();
  await tester.tap(find.text('Confirm'));
  await tester.pumpAndSettle();
}

class _RecordingNavigatorObserver extends NavigatorObserver {
  final pushed = <String?>[];
  final replaced = <(String?, String?)>[];

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pushed.add(route.settings.name);
  }

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    replaced.add((oldRoute?.settings.name, newRoute?.settings.name));
  }
}

void main() {
  testWidgets(
    'the device step offers the paired servers that approved this device',
    (tester) async {
      final broadcast = _Broadcast(
        pairedServers: [
          _server('Bedroom', ClientStatus.approved),
          _server('Kitchen', ClientStatus.pending),
          _server('Garage', ClientStatus.blocked),
        ],
      );
      await _openDeviceStep(tester, broadcast);

      expect(broadcast.users, isEmpty);
      expect(broadcast.pairedServerFetches, 1);
      expect(find.text('Bedroom'), findsOneWidget);
      expect(find.text('Kitchen'), findsNothing);
      expect(find.text('Garage'), findsNothing);
      expect(find.text('Search Neighbors'), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets(
    'pairing from the device step lists the device once it is approved',
    (tester) async {
      final broadcast = _Broadcast();
      await _openDeviceStep(
        tester,
        broadcast,
        searchNeighbors: (_) async => broadcast.nextPairedServers = [
          _server('Bedroom', ClientStatus.approved),
        ],
      );
      expect(find.text('Bedroom'), findsNothing);

      await tester.tap(find.text('Search Neighbors'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Confirm'));
      await tester.pumpAndSettle();

      expect(broadcast.pairedServerFetches, 2);
      expect(find.text('Bedroom'), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets(
    'going back and confirming Rune Device again refreshes the paired servers',
    (tester) async {
      final broadcast = _Broadcast();
      await _openDeviceStep(tester, broadcast);
      expect(broadcast.pairedServerFetches, 1);
      expect(find.text('Bedroom'), findsNothing);

      broadcast.nextPairedServers = [_server('Bedroom', ClientStatus.approved)];
      await tester.tap(find.text('Back'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Confirm'));
      await tester.pumpAndSettle();

      expect(broadcast.pairedServerFetches, 2);
      expect(find.text('Bedroom'), findsOneWidget);
      await tester.pump(const Duration(seconds: 1));
    },
  );

  testWidgets('Rune Device opens the paired-device step in the guide', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pairedDevices: const [
                  ClientSummary(
                    alias: 'Bedroom',
                    fingerprint: 'peer',
                    deviceModel: 'Desktop',
                    status: ClientStatus.approved,
                  ),
                ],
                searchNeighbors: (_) async {},
                addNeighborManually: (_) async => null,
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rune Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();

    expect(find.text('Search Neighbors'), findsOneWidget);
    expect(find.text('Add Neighbor Manually'), findsOneWidget);
    expect(find.text('Bedroom'), findsOneWidget);
    expect(find.text('Cancel'), findsOneWidget);
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets(
    'Back from local Library Setup returns to the source list and Cancel closes the guide',
    (tester) async {
      final registry = _emptyRegistry();
      final manager = _testManager();
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                dependencies: AddLibraryGuideDependencies(
                  pickLocalFolder: (_) async => '/picked/music',
                  testLocalLibrary: (_) async => false,
                ),
              ),
              child: const Text('Open guide'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open guide'));
      await tester.pumpAndSettle();
      expect(find.text('Local Folder'), findsOneWidget);
      expect(find.text('Rune Device'), findsOneWidget);
      expect(find.text('WebDAV'), findsOneWidget);
      expect(find.text('S3'), findsOneWidget);
      expect(find.text('Subsonic'), findsOneWidget);
      expect(find.byType(ListTile), findsNWidgets(5));
      // Sources added later stay below the original two rows.
      final rows = [
        'Local Folder',
        'Rune Device',
        'WebDAV',
        'S3',
        'Subsonic',
      ].map((label) => tester.getTopLeft(find.text(label)).dy).toList();
      for (var index = 1; index < rows.length; index++) {
        expect(rows[index], greaterThan(rows[index - 1]));
      }
      expect(find.text('Confirm'), findsNothing);

      await tester.tap(find.text('Local Folder'));
      await tester.pumpAndSettle();
      expect(find.text('Confirm'), findsOneWidget);
      await tester.tap(find.text('Confirm'));
      await tester.pumpAndSettle();
      expect(find.text('Library Setup'), findsOneWidget);

      await tester.tap(find.text('Back'));
      await tester.pumpAndSettle();
      expect(find.text('Local Folder'), findsOneWidget);
      expect(find.text('Rune Device'), findsOneWidget);
      expect(find.text('Back'), findsNothing);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Local Folder'), findsNothing);
    },
  );

  for (final (choice, expectedMode) in [
    ('Local hard drive', LibraryInitializeMode.portable),
    ('In the cloud', LibraryInitializeMode.redirected),
  ]) {
    testWidgets('new local folder asks for a mode inside the guide: $choice', (
      tester,
    ) async {
      final modes = <LibraryInitializeMode?>[];
      final steps = <String>[];
      final scans = <String>[];
      final registry = LibraryRegistryProvider(
        loadRecords: () async => ListLibraryRecordsResponse(
          records: const [],
          invalidRecords: const [],
          lastOpenRecord: null,
        ),
        saveBookmark: (path) async => steps.add('bookmark $path'),
        registerLocal: (path, alias, mode) async {
          expect(path, '/picked/music');
          expect(alias, 'music');
          modes.add(mode);
          return mode == null
              ? const RegisterLibraryResponse(
                  outcome: RegisterLibraryOutcomeNeedsInitializeMode(),
                )
              : RegisterLibraryResponse(
                  record: _record(),
                  outcome: const RegisterLibraryOutcomeRegistered(),
                );
        },
        openRecord: (recordId, callId) async {
          steps.add('open $recordId');
          return _opened(recordId, callId);
        },
      );
      final manager = LibraryManagerProvider(
        sendScanRequest: (_) {},
        navigate: (_) {},
      );
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                dependencies: AddLibraryGuideDependencies(
                  pickLocalFolder: (_) async => '/picked/music',
                  testLocalLibrary: (_) async => false,
                  registerLocal: (path, alias, mode) async {
                    expect(path, '/picked/music');
                    expect(alias, 'music');
                    modes.add(mode);
                    return RegisterLibraryResponse(
                      record: _record(),
                      outcome: const RegisterLibraryOutcomeRegistered(),
                    );
                  },
                  openRecord: (recordId) async => steps.add('open $recordId'),
                  scanLibrary: (recordId) async => scans.add(recordId),
                ),
              ),
              child: const Text('Open guide'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open guide'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Local Folder'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Confirm'));
      await tester.pumpAndSettle();
      expect(find.text('Library Setup'), findsOneWidget);
      expect(find.text('In the cloud'), findsOneWidget);
      expect(
        find.text('We will store your index in the configuration directory.'),
        findsOneWidget,
      );
      // One tap chooses, as in the Library Setup dialog.
      await tester.tap(find.text(choice));
      await tester.pumpAndSettle();

      expect(modes, [expectedMode]);
      expect(steps, ['bookmark /picked/music', 'open record-1']);
      expect(scans, hasLength(1));
      expect(scans, ['record-1']);
    });
  }

  testWidgets('initialized local folder skips the mode question', (
    tester,
  ) async {
    final modes = <LibraryInitializeMode?>[];
    final steps = <String>[];
    final scans = <String>[];
    final registry = _emptyRegistry(
      saveBookmark: (path) async => steps.add('bookmark $path'),
    );
    final manager = _testManager();
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pickLocalFolder: (_) async => '/picked/music',
                testLocalLibrary: (_) async => true,
                registerLocal: (_, __, mode) async {
                  modes.add(mode);
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => steps.add('open $recordId'),
                scanLibrary: (recordId) async => scans.add(recordId),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Local Folder'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(find.text('Library Setup'), findsNothing);
    expect(modes, [null]);
    expect(steps, ['bookmark /picked/music', 'open record-1']);
    expect(scans, isEmpty);
  });

  testWidgets('a local folder that cannot be checked shows the failure', (
    tester,
  ) async {
    final registry = _emptyRegistry();
    final manager = _testManager();
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pickLocalFolder: (_) async => '/picked/music',
                testLocalLibrary: (_) async => throw const RequestFailure(
                  'TestLibraryInitializedRequest',
                  RequestErrorKind.unavailable,
                  'library folder unavailable',
                ),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Local Folder'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(find.text('Unable to Open Library'), findsOneWidget);
    expect(find.text('This is not available right now.'), findsOneWidget);
  });

  testWidgets('guide pairing rows call the existing pairing flows', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    var searchOpened = false;
    var manualOpened = false;
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                searchNeighbors: (_) async => searchOpened = true,
                addNeighborManually: (_) async {
                  manualOpened = true;
                  return null;
                },
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rune Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Search Neighbors'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add Neighbor Manually'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    expect(searchOpened, isTrue);
    expect(manualOpened, isTrue);
  });

  testWidgets('peer libraries enable actions from sync and control grants', (
    tester,
  ) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    const device = ClientSummary(
      alias: 'Bedroom',
      fingerprint: 'peer',
      deviceModel: 'Desktop',
      status: ClientStatus.approved,
    );
    const libraries = [
      PeerLibrary(
        libraryId: 'read-only',
        alias: 'Read Only',
        added: false,
        connection: DeviceConnectionLevel.view,
        sync: DeviceSyncLevel.pull,
      ),
      PeerLibrary(
        libraryId: 'sync',
        alias: 'Syncable',
        added: false,
        connection: DeviceConnectionLevel.viewEdit,
        sync: DeviceSyncLevel.pullPush,
      ),
      PeerLibrary(
        libraryId: 'hidden',
        alias: 'Not Shared',
        added: false,
        connection: DeviceConnectionLevel.none,
        sync: DeviceSyncLevel.none,
      ),
    ];
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pairedDevices: const [device],
                loadLibraries: (_) async =>
                    const ListPeerLibrariesResponse(libraries: libraries),
                loadGrants: (_) async => const GetDeviceGrantsResponse(
                  grants: DeviceGrantsInfo(
                    control: false,
                    libraries: [],
                    administer: false,
                  ),
                  libraries: [],
                ),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rune Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Read Only').first);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(find.text('No Playback Control'), findsWidgets);
    expect(find.text('How do you want to use it?'), findsOneWidget);
    await tester.tap(find.text('Sync to This Device'));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNotNull,
    );
    await tester.tap(find.text('Control Playback'));
    await tester.pumpAndSettle();
    expect(find.text('No Playback Control'), findsWidgets);
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNull,
    );
    await tester.tap(find.text('Back'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Not Shared').first);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Sync to This Device'));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNull,
    );
    await tester.pump(const Duration(seconds: 1));
  });

  testWidgets('Sync adds the replica then opens its record', (tester) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    const device = ClientSummary(
      alias: 'Bedroom',
      fingerprint: 'peer',
      deviceModel: 'Desktop',
      status: ClientStatus.approved,
    );
    final calls = <String>[];
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pairedDevices: const [device],
                loadLibraries: (_) async => const ListPeerLibrariesResponse(
                  libraries: [
                    PeerLibrary(
                      libraryId: 'shared',
                      alias: 'Shared Music',
                      added: false,
                      connection: DeviceConnectionLevel.view,
                      sync: DeviceSyncLevel.pull,
                    ),
                  ],
                ),
                loadGrants: (_) async => const GetDeviceGrantsResponse(
                  grants: DeviceGrantsInfo(
                    control: false,
                    libraries: [],
                    administer: false,
                  ),
                  libraries: [],
                ),
                addLibrary: (library, fingerprint) async {
                  calls.add('add:${library.libraryId}:$fingerprint');
                  return const AddRemoteLibraryResponse(
                    recordId: 'replica-record',
                  );
                },
                openRecord: (recordId) async => calls.add('open:$recordId'),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rune Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Shared Music'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Sync to This Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();

    expect(calls, ['add:shared:peer', 'open:replica-record']);
  });

  testWidgets('Control Playback registers remote control', (tester) async {
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    const device = ClientSummary(
      alias: 'Bedroom',
      fingerprint: 'peer',
      deviceModel: 'Desktop',
      status: ClientStatus.approved,
    );
    final calls = <String>[];
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                pairedDevices: const [device],
                loadLibraries: (_) async => const ListPeerLibrariesResponse(
                  libraries: [
                    PeerLibrary(
                      libraryId: 'shared',
                      alias: 'Shared Music',
                      added: false,
                      connection: DeviceConnectionLevel.view,
                      sync: DeviceSyncLevel.pull,
                    ),
                  ],
                ),
                loadGrants: (_) async => const GetDeviceGrantsResponse(
                  grants: DeviceGrantsInfo(
                    control: true,
                    libraries: [],
                    administer: false,
                  ),
                  libraries: [],
                ),
                registerControl: (fingerprint, alias) async {
                  calls.add('register:$fingerprint:$alias');
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => calls.add('open:$recordId'),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Rune Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Bedroom'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Shared Music'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Control Playback'));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNotNull,
    );
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();

    expect(calls, ['register:peer:Bedroom', 'open:record-1']);
  });

  testWidgets('search pairing trusts and registers the device only', (
    tester,
  ) async {
    final registryCalls = <String>[];
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
      registerRemote: (fingerprint, alias) async {
        registryCalls.add('remote:$fingerprint:$alias');
        return RegisterLibraryResponse(
          record: _record(),
          outcome: const RegisterLibraryOutcomeRegistered(),
        );
      },
    );
    final manager = LibraryManagerProvider(sendScanRequest: (_) {});
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    final events = <String>[];
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => pairRuneDevice(
              context,
              showSearch: (_) async => (
                const DiscoveredDeviceMessage(
                  alias: 'Bedroom',
                  deviceModel: 'Desktop',
                  deviceType: DeviceTypeMessage.desktop,
                  fingerprint: 'peer',
                  lastSeenUnixEpoch: 0,
                  addresses: ['192.0.2.1:9999'],
                ),
                true,
              ),
              trustDevice: (fingerprint, hosts) async =>
                  events.add('trust:$fingerprint:${hosts.single}'),
              registerDevice: (hosts, fingerprint) async =>
                  events.add('pair:$fingerprint:${hosts.single}'),
            ),
            child: const Text('Pair only'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Pair only'));
    await tester.pumpAndSettle();

    expect(events, ['trust:peer:192.0.2.1:9999', 'pair:peer:192.0.2.1:9999']);
    expect(registryCalls, isEmpty);
    await tester.pump(const Duration(seconds: 1));
  });

  test('Devices page and manual pairing do not register a library', () {
    final devices = File(
      'lib/screens/settings_devices/settings_devices.dart',
    ).readAsStringSync();
    final manualPairing = File(
      'lib/screens/settings_neighbors/widgets/add_remote_device_dialog.dart',
    ).readAsStringSync();
    final searchPairing = File(
      'lib/screens/settings_neighbors/widgets/search_remote_device_setting_button.dart',
    ).readAsStringSync();

    expect(devices, isNot(contains('PeerLibrariesDialog')));
    expect(devices, isNot(contains('deviceLibraries')));
    expect(manualPairing, isNot(contains('registerRemoteControl')));
    expect(manualPairing, isNot(contains('.open(')));
    expect(searchPairing, contains('pairRuneDevice(context)'));
  });

  testWidgets(
    'WebDAV form validates fields, reports failures, and opens on success',
    (tester) async {
      final registry = _emptyRegistry();
      final manager = _testManager();
      final opened = <String>[];
      final requests = <(OpenDalLibraryService, String, ShareDataChoice)>[];
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);
      var fail = true;
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                dependencies: AddLibraryGuideDependencies(
                  detectOpenDal: (_) async =>
                      const DetectOpenDalShareDataResponse(
                        result: OpenDalShareDataDetectionWritableEmpty(),
                      ),
                  registerOpenDal: (service, alias, choice) async {
                    requests.add((service, alias, choice));
                    if (fail) {
                      throw const RequestFailure(
                        'RegisterOpenDalLibraryRequest',
                        RequestErrorKind.unavailable,
                        'Could not reach the remote library',
                        RequestFailureCauseLibraryRegistrationFailed(
                          kind: RegisterLibraryFailureKind.openDalUnreachable,
                        ),
                      );
                    }
                    return RegisterLibraryResponse(
                      record: _record(),
                      outcome: const RegisterLibraryOutcomeRegistered(),
                    );
                  },
                  openRecord: (recordId) async => opened.add(recordId),
                  scanLibrary: (recordId) async => opened.add('scan:$recordId'),
                ),
              ),
              child: const Text('Open guide'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open guide'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('WebDAV'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();

      expect(find.byType(TextBox), findsNWidgets(4));
      expect(find.text('Server'), findsOneWidget);
      expect(find.text('Folder'), findsOneWidget);
      expect(find.text('Username'), findsOneWidget);
      expect(find.text('Password'), findsOneWidget);
      final confirm = tester.widget<Button>(
        find.widgetWithText(Button, 'Confirm'),
      );
      expect(confirm.onPressed, isNull);
      final webDavValues = [
        'https://dav.example.test',
        '/music',
        'alice',
        'secret',
      ];
      for (var index = 0; index < webDavValues.length; index++) {
        await tester.enterText(
          find.byType(TextBox).at(index),
          webDavValues[index],
        );
      }
      await tester.pumpAndSettle();
      expect(
        tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
        isNotNull,
      );
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      expect(find.text('Where should Rune store your index?'), findsOneWidget);
      await tester.tap(find.text('On this device'));
      await tester.pumpAndSettle();
      expect(
        find.text('Check the server address and network connection.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Back'));
      await tester.pumpAndSettle();
      expect(find.byType(TextBox), findsNWidgets(4));
      expect(requests.single.$1, isA<OpenDalLibraryServiceWebDav>());
      expect(requests.single.$2, 'music');
      expect(requests.single.$3, ShareDataChoice.thisDeviceOnly);

      fail = false;
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('On this device'));
      await tester.pumpAndSettle();
      expect(opened, ['record-1', 'scan:record-1']);
      expect(find.text('WebDAV'), findsNothing);
    },
  );

  testWidgets('S3 form validates fields and registers then opens its record', (
    tester,
  ) async {
    final registry = _emptyRegistry();
    final manager = _testManager();
    final opened = <String>[];
    OpenDalLibraryService? registeredService;
    String? registeredAlias;
    ShareDataChoice? registeredChoice;
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                detectOpenDal: (_) async =>
                    const DetectOpenDalShareDataResponse(
                      result: OpenDalShareDataDetectionWritableEmpty(),
                    ),
                registerOpenDal: (service, alias, choice) async {
                  registeredService = service;
                  registeredAlias = alias;
                  registeredChoice = choice;
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => opened.add(recordId),
                scanLibrary: (recordId) async => opened.add('scan:$recordId'),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('S3'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();

    expect(find.byType(TextBox), findsNWidgets(6));
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNull,
    );
    for (final entry in <(int, String)>[
      (0, 'https://s3.example.test'),
      (1, 'music-bucket'),
      (2, 'us-east-1'),
      (3, '/library'),
      (4, 'access'),
      (5, 'secret'),
    ]) {
      await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
    }
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('In the shared folder'));
    await tester.pumpAndSettle();
    expect(registeredService, isA<OpenDalLibraryServiceS3>());
    expect(registeredAlias, 'music-bucket');
    expect(registeredChoice, ShareDataChoice.keepInShare);
    expect(opened, ['record-1', 'scan:record-1']);
  });

  testWidgets('existing and read-only shares skip the data placement step', (
    tester,
  ) async {
    for (final detection in <OpenDalShareDataDetection>[
      const OpenDalShareDataDetectionExistingLibrary(libraryId: 'existing-id'),
      const OpenDalShareDataDetectionReadOnly(),
    ]) {
      final registry = _emptyRegistry();
      final manager = _testManager();
      final choices = <ShareDataChoice>[];
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                dependencies: AddLibraryGuideDependencies(
                  detectOpenDal: (_) async =>
                      DetectOpenDalShareDataResponse(result: detection),
                  registerOpenDal: (_, _, choice) async {
                    choices.add(choice);
                    return RegisterLibraryResponse(
                      record: _record(),
                      outcome: const RegisterLibraryOutcomeRegistered(),
                    );
                  },
                  openRecord: (_) async {},
                  scanLibrary: (_) async {},
                ),
              ),
              child: const Text('Open guide'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open guide'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('WebDAV'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      for (var index = 0; index < 4; index++) {
        await tester.enterText(find.byType(TextBox).at(index), 'value');
      }
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      expect(find.text('Where should Rune store your index?'), findsNothing);
      expect(find.text('In the shared folder'), findsNothing);
      expect(find.text('On this device'), findsNothing);
      expect(choices, [ShareDataChoice.thisDeviceOnly]);
    }
  });

  testWidgets('Subsonic offers both uses and registers the selected player', (
    tester,
  ) async {
    final registry = _emptyRegistry();
    final manager = _testManager();
    final requests = <RegisterSubsonicLibraryRequest>[];
    final opened = <String>[];
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                registerSubsonic: (request) async {
                  requests.add(request);
                  if (request.mode == null) {
                    return const RegisterLibraryResponse(
                      outcome: RegisterLibraryOutcomeNeedsSubsonicMode(
                        allowed: [
                          SubsonicOpenModeMessage.playOnThisDevice,
                          SubsonicOpenModeMessage.remoteControl,
                        ],
                      ),
                    );
                  }
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => opened.add(recordId),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Subsonic'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    for (final entry in <(int, String)>[
      (0, 'https://music.example.test'),
      (1, 'alice'),
      (2, 'secret'),
    ]) {
      await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
    }
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(find.text('Play on This Device'), findsOneWidget);
    expect(find.text('How do you want to use it?'), findsOneWidget);
    expect(find.text("Control the Server's Player"), findsOneWidget);
    await tester.tap(find.text('Back'));
    await tester.pumpAndSettle();
    expect(find.text('https://music.example.test'), findsOneWidget);
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text("Control the Server's Player"));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(requests.map((request) => request.mode), [
      null,
      null,
      SubsonicOpenModeMessage.remoteControl,
    ]);
    expect(requests.first.endpoint, 'https://music.example.test');
    expect(requests.first.username, 'alice');
    expect(requests.first.password, 'secret');
    expect(opened, ['record-1']);
  });

  testWidgets('Subsonic disables jukebox control without its role', (
    tester,
  ) async {
    final registry = _emptyRegistry();
    final manager = _testManager();
    final requests = <RegisterSubsonicLibraryRequest>[];
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                registerSubsonic: (request) async {
                  requests.add(request);
                  if (request.mode == null) {
                    return const RegisterLibraryResponse(
                      outcome: RegisterLibraryOutcomeNeedsSubsonicMode(
                        allowed: [SubsonicOpenModeMessage.playOnThisDevice],
                      ),
                    );
                  }
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (_) async {},
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Subsonic'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    for (final entry in <(int, String)>[
      (0, 'https://music.example.test'),
      (1, 'alice'),
      (2, 'secret'),
    ]) {
      await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
    }
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    await tester.tap(find.text("Control the Server's Player"));
    await tester.pumpAndSettle();
    expect(
      find.text('Your account is not allowed to do this.'),
      findsOneWidget,
    );
    expect(
      tester.widget<Button>(find.widgetWithText(Button, 'Confirm')).onPressed,
      isNull,
    );
    await tester.tap(find.text('Play on This Device'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(requests.map((request) => request.mode), [
      null,
      SubsonicOpenModeMessage.playOnThisDevice,
    ]);
  });

  testWidgets('Subsonic registration errors use library registry messages', (
    tester,
  ) async {
    final registry = _emptyRegistry();
    final manager = _testManager();
    addTearDown(registry.dispose);
    addTearDown(manager.dispose);
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              dependencies: AddLibraryGuideDependencies(
                registerSubsonic: (_) async => throw const RequestFailure(
                  'RegisterSubsonicLibraryRequest',
                  RequestErrorKind.permissionDenied,
                  'The Subsonic credentials were rejected',
                  RequestFailureCauseLibraryRegistrationFailed(
                    kind: RegisterLibraryFailureKind.subsonicWrongCredentials,
                  ),
                ),
              ),
            ),
            child: const Text('Open guide'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open guide'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Subsonic'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    for (final entry in <(int, String)>[
      (0, 'https://music.example.test'),
      (1, 'alice'),
      (2, 'wrong'),
    ]) {
      await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
    }
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(find.text('Check the username and password.'), findsOneWidget);
  });

  testWidgets(
    'Subsonic added from the welcome page navigates root to /library',
    (tester) async {
      cafeMode = false;
      $history.reset();
      $router.update('/', null);
      addTearDown(() {
        $history.reset();
        $router.update('/library', null);
      });
      final registry = _emptyRegistry();
      final manager = _testManager();
      final opened = <String>[];
      final rootRoutes = <String>[];
      final rootObserver = _RecordingNavigatorObserver();
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);

      const subsonicRecord = LibraryRecordSummary(
        shareSync: false,
        recordId: 'subsonic-1',
        alias: '127.0.0.1',
        kind: LibraryRecordKindSummarySubsonic(
          endpoint: 'http://127.0.0.1:4533',
          account: 'alice',
        ),
      );

      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final broadcast = _Broadcast();
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(broadcast.dispose);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: sizes),
            ChangeNotifierProvider.value(value: responsive),
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider<LibraryManagerProvider>.value(
              value: manager,
            ),
            ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
          ],
          child: FluentApp(
            navigatorKey: rootNavigatorKey,
            navigatorObservers: [rootObserver],
            initialRoute: '/',
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            onGenerateRoute: (settings) {
              final name = settings.name!;
              rootRoutes.add(name);
              return FluentPageRoute<void>(
                settings: settings,
                builder: (context) => name == '/library'
                    ? const Text('Library Home')
                    : Button(
                        onPressed: () => showAddLibraryGuide(
                          context,
                          dependencies: AddLibraryGuideDependencies(
                            registerSubsonic: (request) async {
                              if (request.mode == null) {
                                return const RegisterLibraryResponse(
                                  outcome:
                                      RegisterLibraryOutcomeNeedsSubsonicMode(
                                        allowed: [
                                          SubsonicOpenModeMessage
                                              .playOnThisDevice,
                                        ],
                                      ),
                                );
                              }
                              return const RegisterLibraryResponse(
                                record: subsonicRecord,
                                outcome: RegisterLibraryOutcomeRegistered(),
                              );
                            },
                            openRecord: (recordId) async =>
                                opened.add(recordId),
                          ),
                        ),
                        child: const Text('Add Library'),
                      ),
              );
            },
          ),
        ),
      );

      await tester.tap(find.text('Add Library'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subsonic'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      for (final entry in <(int, String)>[
        (0, 'http://127.0.0.1:4533'),
        (1, 'alice'),
        (2, 'secret'),
      ]) {
        await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
      }
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Play on This Device'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();

      expect(opened, ['subsonic-1']);
      expect(find.text('Library Home'), findsOneWidget);
      expect(find.text('Add Library'), findsNothing);
      expect(rootRoutes, ['/', '/library']);
      expect(rootObserver.replaced, [('/', '/library')]);
      expect(rootNavigatorKey.currentState!.canPop(), isFalse);
      expect($router.path, '/library');
    },
  );

  testWidgets(
    'Subsonic added from inside a library pushes /library on the library navigator',
    (tester) async {
      cafeMode = false;
      $history.reset();
      $router.update('/settings/library', null);
      addTearDown(() {
        $history.reset();
        $router.update('/library', null);
      });
      final registry = _emptyRegistry();
      final manager = _testManager();
      final opened = <String>[];
      final rootRoutes = <String>[];
      final nestedRoutes = <String>[];
      final nestedObserver = _RecordingNavigatorObserver();
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);

      const subsonicRecord = LibraryRecordSummary(
        shareSync: false,
        recordId: 'subsonic-2',
        alias: '127.0.0.1',
        kind: LibraryRecordKindSummarySubsonic(
          endpoint: 'http://127.0.0.1:4533',
          account: 'alice',
        ),
      );

      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final broadcast = _Broadcast();
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(broadcast.dispose);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: sizes),
            ChangeNotifierProvider.value(value: responsive),
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider<LibraryManagerProvider>.value(
              value: manager,
            ),
            ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
          ],
          child: FluentApp(
            navigatorKey: rootNavigatorKey,
            initialRoute: '/',
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            onGenerateRoute: (settings) {
              rootRoutes.add(settings.name!);
              return FluentPageRoute<void>(
                settings: settings,
                builder: (_) => Navigator(
                  key: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
                  observers: [nestedObserver],
                  initialRoute: '/',
                  onGenerateRoute: (nestedSettings) {
                    final name = nestedSettings.name!;
                    nestedRoutes.add(name);
                    return FluentPageRoute<void>(
                      settings: nestedSettings,
                      builder: (context) => name == '/library'
                          ? const Text('Nested Library Home')
                          : Button(
                              onPressed: () => showAddLibraryGuide(
                                context,
                                dependencies: AddLibraryGuideDependencies(
                                  registerSubsonic: (request) async {
                                    if (request.mode == null) {
                                      return const RegisterLibraryResponse(
                                        outcome:
                                            RegisterLibraryOutcomeNeedsSubsonicMode(
                                              allowed: [
                                                SubsonicOpenModeMessage
                                                    .playOnThisDevice,
                                              ],
                                            ),
                                      );
                                    }
                                    return const RegisterLibraryResponse(
                                      record: subsonicRecord,
                                      outcome:
                                          RegisterLibraryOutcomeRegistered(),
                                    );
                                  },
                                  openRecord: (recordId) async =>
                                      opened.add(recordId),
                                ),
                              ),
                              child: const Text('Add Library'),
                            ),
                    );
                  },
                ),
              );
            },
          ),
        ),
      );

      await tester.tap(find.text('Add Library'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subsonic'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      for (final entry in <(int, String)>[
        (0, 'http://127.0.0.1:4533'),
        (1, 'alice'),
        (2, 'secret'),
      ]) {
        await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
      }
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Play on This Device'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();

      expect(opened, ['subsonic-2']);
      expect(find.text('Nested Library Home'), findsOneWidget);
      expect(rootRoutes, ['/']);
      expect(nestedRoutes, ['/', '/library']);
      expect(nestedObserver.replaced, isEmpty);
      expect(nestedObserver.pushed, ['/', '/library']);
      expect(
        runeWithNavigationBarAndPlaybackControllorNavigatorKey
            .currentState!
            .canPop(),
        isTrue,
      );
      expect($router.path, '/library');
    },
  );

  testWidgets(
    'failed Subsonic registration from the welcome page shows its error without navigating',
    (tester) async {
      cafeMode = false;
      $history.reset();
      $router.update('/', null);
      addTearDown(() {
        $history.reset();
        $router.update('/library', null);
      });
      final registry = _emptyRegistry();
      final manager = _testManager();
      final rootRoutes = <String>[];
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);

      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final broadcast = _Broadcast();
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(broadcast.dispose);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: sizes),
            ChangeNotifierProvider.value(value: responsive),
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider<LibraryManagerProvider>.value(
              value: manager,
            ),
            ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
          ],
          child: FluentApp(
            navigatorKey: rootNavigatorKey,
            initialRoute: '/',
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            onGenerateRoute: (settings) {
              final name = settings.name!;
              rootRoutes.add(name);
              return FluentPageRoute<void>(
                settings: settings,
                builder: (context) => name == '/library'
                    ? const Text('Library Home')
                    : Button(
                        onPressed: () => showAddLibraryGuide(
                          context,
                          dependencies: AddLibraryGuideDependencies(
                            registerSubsonic: (_) async =>
                                throw const RequestFailure(
                                  'RegisterSubsonicLibraryRequest',
                                  RequestErrorKind.permissionDenied,
                                  'The Subsonic credentials were rejected',
                                  RequestFailureCauseLibraryRegistrationFailed(
                                    kind: RegisterLibraryFailureKind
                                        .subsonicWrongCredentials,
                                  ),
                                ),
                          ),
                        ),
                        child: const Text('Add Library'),
                      ),
              );
            },
          ),
        ),
      );

      await tester.tap(find.text('Add Library'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subsonic'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      for (final entry in <(int, String)>[
        (0, 'http://127.0.0.1:4533'),
        (1, 'alice'),
        (2, 'wrong'),
      ]) {
        await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
      }
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();

      expect(find.text('Check the username and password.'), findsOneWidget);
      expect(find.text('Library Home'), findsNothing);
      expect(rootRoutes, ['/']);
    },
  );

  testWidgets(
    'dismissing the guide while opening a record suppresses late navigation',
    (tester) async {
      cafeMode = false;
      $history.reset();
      $router.update('/', null);
      addTearDown(() {
        $history.reset();
        $router.update('/library', null);
      });
      final registry = _emptyRegistry();
      final manager = _testManager();
      final rootRoutes = <String>[];
      final openCompleter = Completer<void>();
      addTearDown(registry.dispose);
      addTearDown(manager.dispose);

      const subsonicRecord = LibraryRecordSummary(
        shareSync: false,
        recordId: 'subsonic-3',
        alias: '127.0.0.1',
        kind: LibraryRecordKindSummarySubsonic(
          endpoint: 'http://127.0.0.1:4533',
          account: 'alice',
        ),
      );

      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      final broadcast = _Broadcast();
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      addTearDown(broadcast.dispose);

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider.value(value: sizes),
            ChangeNotifierProvider.value(value: responsive),
            ChangeNotifierProvider<LibraryRegistryProvider>.value(
              value: registry,
            ),
            ChangeNotifierProvider<LibraryManagerProvider>.value(
              value: manager,
            ),
            ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
          ],
          child: FluentApp(
            navigatorKey: rootNavigatorKey,
            initialRoute: '/',
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            onGenerateRoute: (settings) {
              final name = settings.name!;
              rootRoutes.add(name);
              return FluentPageRoute<void>(
                settings: settings,
                builder: (context) => name == '/library'
                    ? const Text('Library Home')
                    : Button(
                        onPressed: () => showAddLibraryGuide(
                          context,
                          dependencies: AddLibraryGuideDependencies(
                            registerSubsonic: (request) async {
                              if (request.mode == null) {
                                return const RegisterLibraryResponse(
                                  outcome:
                                      RegisterLibraryOutcomeNeedsSubsonicMode(
                                        allowed: [
                                          SubsonicOpenModeMessage
                                              .playOnThisDevice,
                                        ],
                                      ),
                                );
                              }
                              return const RegisterLibraryResponse(
                                record: subsonicRecord,
                                outcome: RegisterLibraryOutcomeRegistered(),
                              );
                            },
                            openRecord: (_) => openCompleter.future,
                          ),
                        ),
                        child: const Text('Add Library'),
                      ),
              );
            },
          ),
        ),
      );

      await tester.tap(find.text('Add Library'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Subsonic'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      for (final entry in <(int, String)>[
        (0, 'http://127.0.0.1:4533'),
        (1, 'alice'),
        (2, 'secret'),
      ]) {
        await tester.enterText(find.byType(TextBox).at(entry.$1), entry.$2);
      }
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Play on This Device'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pump();

      await tester.tap(find.widgetWithText(Button, 'Cancel'));
      await tester.pumpAndSettle();
      openCompleter.complete();
      await tester.pumpAndSettle();

      expect(find.text('Library Home'), findsNothing);
      expect(find.text('Add Library'), findsOneWidget);
      expect(rootRoutes, ['/']);
      expect($router.path, '/');
    },
  );

  group('Apple Music', () {
    final strings = lookupAppLocalizations(const Locale('en'));
    const appleRecord = LibraryRecordSummary(
      recordId: 'apple-record',
      alias: 'Apple Music',
      kind: LibraryRecordKindSummaryAppleMusic(),
      signedIn: true,
    );
    const authorization = HostAuthorization(
      token: 'user-token',
      developerToken: 'developer-token',
    );

    Future<void> openGuide(
      WidgetTester tester, {
      required LibraryRegistryProvider registry,
      required AddLibraryGuideDependencies dependencies,
    }) async {
      final manager = _testManager();
      addTearDown(manager.dispose);
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () =>
                  showAddLibraryGuide(context, dependencies: dependencies),
              child: const Text('Open guide'),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open guide'));
      await tester.pumpAndSettle();
    }

    Future<void> addAppleMusic(WidgetTester tester) async {
      await tester.tap(find.text(strings.appleMusic));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pumpAndSettle();
    }

    /// A registry whose hub keeps one Apple Music record, as the real one does:
    /// registering again signs that record in again.
    LibraryRegistryProvider registryWith(
      List<String> steps, {
      Object? failure,
    }) {
      var registered = false;
      final registry = LibraryRegistryProvider(
        loadRecords: () async => const ListLibraryRecordsResponse(
          records: [],
          invalidRecords: [],
          lastOpenRecord: null,
        ),
        registerService: (service, alias, signIn) async {
          final platform = signIn as ServiceSignInPlatform;
          steps.add(
            'register ${service.name} $alias '
            '${platform.userToken} ${platform.developerToken}',
          );
          if (failure != null) throw failure;
          steps.add(registered ? 'signed in again' : 'created');
          registered = true;
          return const RegisterLibraryResponse(
            record: appleRecord,
            outcome: RegisterLibraryOutcomeRegistered(),
          );
        },
      );
      addTearDown(registry.dispose);
      return registry;
    }

    AddLibraryGuideDependencies dependenciesWith(
      List<String> steps, {
      HostAuthorizationResult authorization = authorization,
      Future<void>? gate,
    }) => AddLibraryGuideDependencies(
      hosts: {
        AppleMusicHost.family: _FakeHost(authorization, steps, gate: gate),
      },
      openRecord: (recordId) async => steps.add('open $recordId'),
    );

    testWidgets('the row is shown only with a platform host', (tester) async {
      await openGuide(
        tester,
        registry: registryWith([]),
        dependencies: const AddLibraryGuideDependencies(hosts: {}),
      );
      expect(find.text('Subsonic'), findsOneWidget);
      expect(find.text(strings.appleMusic), findsNothing);
      await tester.pumpWidget(const SizedBox());

      await openGuide(
        tester,
        registry: registryWith([]),
        dependencies: dependenciesWith([]),
      );
      expect(find.text(strings.appleMusic), findsOneWidget);
      expect(find.text(strings.appleMusicSubtitle), findsOneWidget);
      // Lets the screen-size provider's debounce timer fire before the test
      // ends.
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('authorizes, registers with both tokens and opens the record', (
      tester,
    ) async {
      final steps = <String>[];
      final registry = registryWith(steps);
      await openGuide(
        tester,
        registry: registry,
        dependencies: dependenciesWith(steps),
      );
      await addAppleMusic(tester);

      expect(steps, [
        'authorize interactive',
        'register appleMusic Apple Music user-token developer-token',
        'created',
        'open apple-record',
      ]);
      expect(registry.records.map((r) => r.recordId), ['apple-record']);
      expect(find.byType(ContentDialog), findsNothing);
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('Confirm is disabled while Apple Music is being added', (
      tester,
    ) async {
      final steps = <String>[];
      final gate = Completer<void>();
      await openGuide(
        tester,
        registry: registryWith(steps),
        dependencies: dependenciesWith(steps, gate: gate.future),
      );
      Button confirm() =>
          tester.widget<Button>(find.widgetWithText(Button, 'Confirm'));
      await tester.tap(find.text(strings.appleMusic));
      await tester.pumpAndSettle();
      expect(confirm().onPressed, isNotNull);

      await tester.tap(find.widgetWithText(Button, 'Confirm'));
      await tester.pump();
      expect(steps, ['authorize interactive']);
      expect(confirm().onPressed, isNull);

      gate.complete();
      await tester.pumpAndSettle();
      expect(steps.last, 'open apple-record');
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('adding it again only signs the same record in again', (
      tester,
    ) async {
      final steps = <String>[];
      final registry = registryWith(steps);
      final dependencies = dependenciesWith(steps);
      for (var round = 0; round < 2; round++) {
        await openGuide(tester, registry: registry, dependencies: dependencies);
        await addAppleMusic(tester);
      }

      expect(steps, [
        'authorize interactive',
        'register appleMusic Apple Music user-token developer-token',
        'created',
        'open apple-record',
        'authorize interactive',
        'register appleMusic Apple Music user-token developer-token',
        'signed in again',
        'open apple-record',
      ]);
      expect(registry.records, hasLength(1));
      await tester.pump(const Duration(seconds: 2));
    });

    for (final (failure, message) in [
      (HostAuthorizationFailure.refused, strings.appleMusicAccessDenied),
      (HostAuthorizationFailure.noToken, strings.appleMusicNoToken),
      (HostAuthorizationFailure.unsupported, strings.appleMusicNeedsNewerMacOS),
      (HostAuthorizationFailure.unavailable, strings.appleMusicNoToken),
    ]) {
      testWidgets('an authorization ${failure.name} registers nothing', (
        tester,
      ) async {
        final steps = <String>[];
        final registry = registryWith(steps);
        await openGuide(
          tester,
          registry: registry,
          dependencies: dependenciesWith(
            steps,
            authorization: HostAuthorizationFailed(failure),
          ),
        );
        await addAppleMusic(tester);

        expect(steps, ['authorize interactive']);
        expect(registry.records, isEmpty);
        expect(find.text(message), findsOneWidget);
        await tester.tap(find.text('Close'));
        await tester.pumpAndSettle();
        // The guide stays open for another try.
        expect(find.text(strings.appleMusic), findsOneWidget);
        await tester.pump(const Duration(seconds: 2));
      });
    }

    for (final (kind, message) in [
      (
        RegisterLibraryFailureKind.remoteNotAuthorized,
        strings.appleMusicSignInFailed,
      ),
      (RegisterLibraryFailureKind.invalidSource, strings.appleMusicNoToken),
      (
        RegisterLibraryFailureKind.remoteUnreachable,
        strings.appleMusicUnreachable,
      ),
      (
        RegisterLibraryFailureKind.remoteProtocol,
        strings.appleMusicUnexpectedResponse,
      ),
      (RegisterLibraryFailureKind.storage, strings.libraryStorageFailure),
    ]) {
      testWidgets('a registration failing with ${kind.name} opens nothing', (
        tester,
      ) async {
        final steps = <String>[];
        await openGuide(
          tester,
          registry: registryWith(
            steps,
            failure: RequestFailure(
              'RegisterServiceLibraryRequest',
              RequestErrorKind.permissionDenied,
              'Library registration failed',
              RequestFailureCauseLibraryRegistrationFailed(kind: kind),
            ),
          ),
          dependencies: dependenciesWith(steps),
        );
        await addAppleMusic(tester);

        expect(steps, hasLength(2));
        expect(steps, isNot(contains(startsWith('open'))));
        expect(find.text(message), findsOneWidget);
        // The tokens never reach the screen.
        expect(find.textContaining('user-token'), findsNothing);
        expect(find.textContaining('developer-token'), findsNothing);
        await tester.pump(const Duration(seconds: 2));
      });
    }
  });
}

class _FakeHost implements AuthorizingHost {
  _FakeHost(this.authorization, this.steps, {this.gate});

  final HostAuthorizationResult authorization;
  final List<String> steps;

  /// What the answer waits for, when the user takes their time.
  final Future<void>? gate;

  @override
  Future<HostAuthorizationResult> authorize({bool interactive = false}) async {
    steps.add(interactive ? 'authorize interactive' : 'authorize');
    await gate;
    return authorization;
  }

  @override
  Stream<void> get accessLost => const Stream.empty();

  @override
  void handle(
    Map<String, dynamic> command,
    void Function(Map<String, dynamic>) emit,
  ) {}
}
