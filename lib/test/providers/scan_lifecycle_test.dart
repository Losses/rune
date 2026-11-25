import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/screens/welcome/scanning.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

class FakeLibraryPath extends ChangeNotifier
    implements LibraryOpenStateProvider {
  @override
  LibraryRecordSummary? get openRecord => const LibraryRecordSummary(
    recordId: 'test-record',
    alias: 'Test',
    kind: LibraryRecordKindSummaryLocal(
      libraryId: 'test-library',
      path: '/test/library',
    ),
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

  test(
    'scan progress and ending follow the reported library task state',
    () async {
      final states = StreamController<LibraryTaskStateChanged>.broadcast(
        sync: true,
      );
      final requests = <ScanAudioLibraryRequest>[];
      final manager = LibraryManagerProvider(
        sendScanRequest: requests.add,
        libraryTaskStates: states.stream,
      );
      addTearDown(states.close);
      addTearDown(manager.dispose);
      const recordId = 'test-record';

      await manager.scanLibrary(recordId, force: true);
      expect(requests, hasLength(1));
      expect(requests.single.force, true);
      expect(requests.single.callId, isNotNull);
      final wait = manager.waitForScanToComplete(recordId);
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanRunning(
            recordId: recordId,
            runId: 'node-run',
            progress: 3,
            total: 8,
            phase: ScanTaskPhase.scanCoverArts,
          ),
        ),
      );
      expect(manager.getScanTaskProgress(recordId)?.progress, 3);
      expect(manager.getScanTaskProgress(recordId)?.total, 8);
      expect(
        manager.getScanTaskProgress(recordId)?.phase,
        ScanTaskPhase.scanCoverArts,
      );
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanEnded(
            recordId: recordId,
            runId: 'node-run',
            progress: 8,
            total: 8,
            phase: ScanTaskPhase.indexFiles,
            outcome: ScanOutcomeFinished(),
          ),
        ),
      );
      expect(
        manager.getScanTaskProgress(recordId)?.status,
        TaskStatus.finished,
      );
      expect(await wait, TaskStatus.finished);
      expect(
        await manager.waitForScanToComplete(recordId),
        TaskStatus.finished,
      );
    },
  );

  test(
    'successful initialization navigates after the reported scan end',
    () async {
      final states = StreamController<LibraryTaskStateChanged>.broadcast(
        sync: true,
      );
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
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanRunning(
            recordId: 'test-record',
            runId: 'node-run',
            progress: 0,
            total: 0,
            phase: ScanTaskPhase.indexFiles,
          ),
        ),
      );
      const ended = LibraryTaskStateChanged(
        state: LibraryTaskStateScanEnded(
          recordId: 'test-record',
          runId: 'node-run',
          progress: 1,
          total: 1,
          phase: ScanTaskPhase.indexFiles,
          outcome: ScanOutcomeFinished(),
        ),
      );
      states
        ..add(ended)
        ..add(ended);
      expect(routes, ['/scanning', '/library']);
    },
  );

  testWidgets('failed initialization screen offers recovery', (tester) async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(
      sync: true,
    );
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: (_) {},
      libraryTaskStates: states.stream,
    );
    addTearDown(states.close);
    addTearDown(manager.dispose);
    await manager.scanLibrary('test-record', isInitializeTask: true);
    states.add(
      const LibraryTaskStateChanged(
        state: LibraryTaskStateScanEnded(
          recordId: 'test-record',
          runId: 'node-run',
          progress: 0,
          total: 0,
          phase: ScanTaskPhase.indexFiles,
          outcome: ScanOutcomeFailed(value: ScanFailure.scanFailed),
        ),
      ),
    );
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

  testWidgets('refused welcome scan start is shown in the scanning page', (
    tester,
  ) async {
    final failures = StreamController<RequestFailedResponse>.broadcast(
      sync: true,
    );
    final requests = <ScanAudioLibraryRequest>[];
    final manager = LibraryManagerProvider(
      sendScanRequest: requests.add,
      navigate: (_) {},
      requestFailures: failures.stream,
    );
    addTearDown(failures.close);
    addTearDown(manager.dispose);
    addTearDown(() async {
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 100));
    });
    await manager.scanLibrary('test-record', isInitializeTask: true);
    failures.add(
      RequestFailedResponse(
        request: 'diagnostic only',
        callId: requests.single.callId,
        kind: RequestErrorKind.conflict,
        detail: 'A scan is already running',
        cause: const RequestFailureCauseLibraryTaskAlreadyRunning(
          task: LibraryTaskKind.scan,
        ),
      ),
    );

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
    expect(
      find.text('Scanning: This is not available right now.'),
      findsOneWidget,
    );
    expect(find.byType(FilledButton), findsOneWidget);
    expect(find.byType(Button), findsOneWidget);
    await tester.pump(const Duration(milliseconds: 100));
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 100));
  });

  testWidgets('scanning screen shows the reported cover-art phase', (
    tester,
  ) async {
    final states = StreamController<LibraryTaskStateChanged>.broadcast(
      sync: true,
    );
    final manager = LibraryManagerProvider(libraryTaskStates: states.stream);
    addTearDown(states.close);
    addTearDown(manager.dispose);
    states.add(
      const LibraryTaskStateChanged(
        state: LibraryTaskStateScanRunning(
          recordId: 'test-record',
          runId: 'node-run',
          progress: 3,
          total: 8,
          phase: ScanTaskPhase.scanCoverArts,
        ),
      ),
    );

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
