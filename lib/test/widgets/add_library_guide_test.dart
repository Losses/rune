import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/broadcast.dart';
import 'package:rune/utils/add_library_flows.dart';
import 'package:rune/utils/dialogs/add_library/add_library_guide.dart';
import 'package:rune/providers/responsive_providers.dart';

LibraryRecordSummary _record() => const LibraryRecordSummary(
  recordId: 'record-1',
  alias: 'music',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library-1', path: '/music'),
);

LibraryRegistryProvider _emptyRegistry() => LibraryRegistryProvider(
  loadRecords: () async => ListLibraryRecordsResponse(
    records: const [],
    invalidRecords: const [],
    lastOpenRecord: null,
  ),
);

LibraryManagerProvider _testManager() =>
    LibraryManagerProvider(sendScanRequest: (_) {});

OpenLibraryResponse _opened(String recordId, int callId) => OpenLibraryResponse(
  callId: Uint64.fromBigInt(BigInt.from(callId)),
  recordId: recordId,
  generation: Uint64.fromBigInt(BigInt.from(1)),
  result: const OpenLibraryResultOpened(),
);

Widget _app({
  required LibraryRegistryProvider registry,
  required LibraryManagerProvider manager,
  required Widget home,
}) {
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);
  final broadcast = _Broadcast();
  addTearDown(broadcast.dispose);
  return MultiProvider(
    providers: [
      ChangeNotifierProvider.value(value: sizes),
      ChangeNotifierProvider.value(value: responsive),
      ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
      ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
      ChangeNotifierProvider<BroadcastProvider>.value(value: broadcast),
    ],
    child: FluentApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: home,
    ),
  );
}

class _Broadcast extends ChangeNotifier implements BroadcastProvider {
  @override
  List<ClientSummary> get users => const [];

  @override
  void fetchUsers() {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
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
    'guide lists all sources, reveals Confirm, and Cancel has no effect',
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
                  pickLocalFolder: () async => null,
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
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Local Folder'), findsNothing);
    },
  );

  testWidgets('new local folder asks for a mode inside the guide', (
    tester,
  ) async {
    final modes = <LibraryInitializeMode?>[];
    final opened = <String>[];
    final scans = <String>[];
    final registry = LibraryRegistryProvider(
      loadRecords: () async => ListLibraryRecordsResponse(
        records: const [],
        invalidRecords: const [],
        lastOpenRecord: null,
      ),
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
        opened.add(recordId);
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
                pickLocalFolder: () async => '/picked/music',
                testLocalLibrary: (_) async => (true, false, null),
                registerLocal: (path, alias, mode) async {
                  expect(path, '/picked/music');
                  expect(alias, 'music');
                  modes.add(mode);
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => opened.add(recordId),
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
    await tester.tap(find.text('Local hard drive'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();

    expect(modes, [LibraryInitializeMode.portable]);
    expect(opened, ['record-1']);
    expect(scans, hasLength(1));
    expect(scans, ['record-1']);
  });

  testWidgets('initialized local folder skips the mode question', (
    tester,
  ) async {
    final modes = <LibraryInitializeMode?>[];
    final opened = <String>[];
    final scans = <String>[];
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
                pickLocalFolder: () async => '/picked/music',
                testLocalLibrary: (_) async => (true, true, null),
                registerLocal: (_, __, mode) async {
                  modes.add(mode);
                  return RegisterLibraryResponse(
                    record: _record(),
                    outcome: const RegisterLibraryOutcomeRegistered(),
                  );
                },
                openRecord: (recordId) async => opened.add(recordId),
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
    expect(opened, ['record-1']);
    expect(scans, isEmpty);
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
                  deviceType: 'Desktop',
                  fingerprint: 'peer',
                  lastSeenUnixEpoch: 0,
                  ips: ['192.0.2.1:9000'],
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

    expect(events, ['trust:peer:192.0.2.1:9000', 'pair:peer:192.0.2.1:9000']);
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
      final requests = <(OpenDalLibraryService, String)>[];
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
                  registerOpenDal: (service, alias) async {
                    requests.add((service, alias));
                    if (fail) {
                      return const RegisterLibraryResponse(
                        outcome: RegisterLibraryOutcomeFailed(
                          value: RegisterLibraryFailureKind.openDalUnreachable,
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
      expect(
        find.text('Check the server address and network connection.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      expect(find.byType(TextBox), findsNWidgets(4));
      expect(requests.single.$1, isA<OpenDalLibraryServiceWebDav>());
      expect(requests.single.$2, 'music');

      fail = false;
      await tester.tap(find.widgetWithText(Button, 'Confirm'));
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
                registerOpenDal: (service, alias) async {
                  registeredService = service;
                  registeredAlias = alias;
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
    expect(registeredService, isA<OpenDalLibraryServiceS3>());
    expect(registeredAlias, 'music-bucket');
    expect(opened, ['record-1', 'scan:record-1']);
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
    await tester.tap(find.text("Control the Server's Player"));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(Button, 'Confirm'));
    await tester.pumpAndSettle();
    expect(requests.map((request) => request.mode), [
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
                registerSubsonic: (_) async => const RegisterLibraryResponse(
                  outcome: RegisterLibraryOutcomeFailed(
                    value: RegisterLibraryFailureKind.subsonicWrongCredentials,
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
}
