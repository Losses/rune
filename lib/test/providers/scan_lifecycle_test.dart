import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/screens/welcome/scanning.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

class FakeLibraryPath extends ChangeNotifier implements LibraryOpenStateProvider {
  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'test-record',
    alias: 'Test',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'test-library', path: '/test/library'),
  );
  @override
  String? get openLocalFolderPath => '/test/library';
  @override
  bool get hasCatalog => true;
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('scan progress and ending follow the reported library task state', () async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(sync: true);
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      libraryTaskStates: states.stream,
    );
    addTearDown(states.close);
    addTearDown(manager.dispose);
    const recordId = 'test-record';

    await manager.scanLibrary(recordId, force: true);
    expect(requests, [const ScanAudioLibraryRequest(force: true)]);
    final wait = manager.waitForScanToComplete(recordId);
    states.add(const LibraryTaskStateChanged(
      state: LibraryTaskStateRunning(
        recordId: recordId,
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 3,
        total: 8,
        scanPhase: ScanTaskPhase.scanCoverArts,
      ),
    ));
    expect(manager.getScanTaskProgress(recordId)?.progress, 3);
    expect(manager.getScanTaskProgress(recordId)?.total, 8);
    expect(
      manager.getScanTaskProgress(recordId)?.phase,
      ScanTaskPhase.scanCoverArts,
    );
    states.add(const LibraryTaskStateChanged(
      state: LibraryTaskStateEnded(
        recordId: recordId,
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 8,
        total: 8,
        outcome: LibraryTaskOutcomeFinished(),
      ),
    ));
    expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.finished);
    expect(await wait, TaskStatus.finished);
    expect(await manager.waitForScanToComplete(recordId), TaskStatus.finished);
  });

  test('successful initialization navigates after the reported scan end', () async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(sync: true);
    final requests = <ScanAudioLibraryRequest>[];
    final routes = <String>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: routes.add,
      libraryTaskStates: states.stream,
    );
    addTearDown(states.close);
    addTearDown(manager.dispose);
    await manager.scanLibrary('test-record', isInitializeTask: true);
    states.add(const LibraryTaskStateChanged(
      state: LibraryTaskStateRunning(
        recordId: 'test-record',
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 0,
        total: 0,
      ),
    ));
    const ended = LibraryTaskStateChanged(
      state: LibraryTaskStateEnded(
        recordId: 'test-record',
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 1,
        total: 1,
        outcome: LibraryTaskOutcomeFinished(),
      ),
    );
    states
      ..add(ended)
      ..add(ended);
    expect(routes, ['/scanning', '/library']);
  });

  testWidgets('failed initialization screen offers recovery', (tester) async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(sync: true);
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: (_) {},
      libraryTaskStates: states.stream,
    );
    addTearDown(states.close);
    addTearDown(manager.dispose);
    await manager.scanLibrary('test-record', isInitializeTask: true);
    states.add(const LibraryTaskStateChanged(
      state: LibraryTaskStateEnded(
        recordId: 'test-record',
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 0,
        total: 0,
        outcome: LibraryTaskOutcomeFailed(value: LibraryTaskError.scanFailed),
      ),
    ));
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => FakeLibraryPath(),
          ),
          ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ],
        child: const FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScanningPage(),
        ),
      ),
    );
    await tester.pump();
    expect(find.textContaining('Scan failed'), findsOneWidget);
    expect(find.byType(FilledButton), findsOneWidget);
    expect(find.byType(Button), findsOneWidget);
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 100));
  });

  testWidgets('scanning screen shows the reported cover-art phase', (tester) async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(sync: true);
    final manager = LibraryManagerProvider(libraryTaskStates: states.stream);
    addTearDown(states.close);
    addTearDown(manager.dispose);
    states.add(const LibraryTaskStateChanged(
      state: LibraryTaskStateRunning(
        recordId: 'test-record',
        task: LibraryTaskKind.scan,
        runId: 'node-run',
        progress: 3,
        total: 8,
        scanPhase: ScanTaskPhase.scanCoverArts,
      ),
    ));

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<LibraryOpenStateProvider>(
            create: (_) => FakeLibraryPath(),
          ),
          ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        ],
        child: const FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScanningPage(),
        ),
      ),
    );
    await tester.pump();
    expect(find.textContaining('album covers collected'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 100));
  });
}
