import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_registry.dart';
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
  return MultiProvider(
    providers: [
      ChangeNotifierProvider.value(value: sizes),
      ChangeNotifierProvider.value(value: responsive),
      ChangeNotifierProvider<LibraryRegistryProvider>.value(value: registry),
      ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
    ],
    child: FluentApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: home,
    ),
  );
}

void main() {
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
      var deviceCalled = false;
      await tester.pumpWidget(
        _app(
          registry: registry,
          manager: manager,
          home: Builder(
            builder: (context) => Button(
              onPressed: () => showAddLibraryGuide(
                context,
                addLocalFolder: (_) async => localCalled = true,
                addRuneDevice: (_) async => deviceCalled = true,
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
      expect(deviceCalled, isFalse);
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
              addRuneDevice: (_) async {},
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

  testWidgets('selecting Rune Device opens the device search flow', (
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
    await tester.pumpWidget(
      _app(
        registry: registry,
        manager: manager,
        home: Builder(
          builder: (context) => Button(
            onPressed: () => showAddLibraryGuide(
              context,
              addLocalFolder: (_) async {},
              addRuneDevice: (context) => addRuneDeviceLibrary(
                context,
                showSearch: (_) async {
                  searchOpened = true;
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
    expect(searchOpened, isTrue);
    expect(find.text('Local Folder'), findsNothing);
  });
}
