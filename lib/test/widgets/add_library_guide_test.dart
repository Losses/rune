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
    'guide lists two sources, reveals Next, and Cancel has no effect',
    (tester) async {
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
      var localCalled = false;
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                addLocalFolder: (_) async => localCalled = true,
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
      expect(find.byType(ListTile), findsNWidgets(2));
      expect(find.text('Confirm'), findsNothing);

      await tester.tap(find.text('Local Folder'));
      await tester.pumpAndSettle();
      expect(find.text('Confirm'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(find.text('Local Folder'), findsNothing);
      expect(localCalled, isFalse);
    },
  );

  testWidgets('selecting Local Folder runs the shared folder flow', (
    tester,
  ) async {
    final modes = <LibraryInitializeMode?>[];
    final opened = <String>[];
    final scans = <ScanAudioLibraryRequest>[];
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
      sendScanRequest: scans.add,
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
              addLocalFolder: (context) => addLocalFolderLibrary(
                context,
                pickDirectory: () async => '/picked/music',
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
    await tester.tap(find.text('Local hard drive'));
    await tester.pumpAndSettle();

    expect(modes, [null, LibraryInitializeMode.portable]);
    expect(opened, ['record-1']);
    expect(scans, hasLength(1));
    expect(scans.single.force, isFalse);
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
              addLocalFolder: (_) async {},
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
    expect(find.text('No Playback Control'), findsWidgets);
    expect(
      tester
          .widget<Button>(find.widgetWithText(Button, 'Sync to This Device'))
          .onPressed,
      isNotNull,
    );
    expect(
      tester
          .widget<Button>(find.widgetWithText(Button, 'Control Playback'))
          .onPressed,
      isNull,
    );

    await tester.tap(find.text('Not Shared').first);
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<Button>(find.widgetWithText(Button, 'Sync to This Device'))
          .onPressed,
      isNull,
    );
    expect(find.text('Not Shared'), findsNWidgets(2));
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
    await tester.tap(find.text('Sync to This Device'));
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
    expect(
      tester
          .widget<Button>(find.widgetWithText(Button, 'Sync to This Device'))
          .onPressed,
      isNotNull,
    );
    expect(find.text('Read Only'), findsOneWidget);
    expect(
      tester
          .widget<Button>(find.widgetWithText(Button, 'Control Playback'))
          .onPressed,
      isNotNull,
    );
    await tester.tap(find.text('Control Playback'));
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
}
