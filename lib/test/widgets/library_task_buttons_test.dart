import 'dart:async';

import 'package:flutter/gestures.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_open_state.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/providers/local_folder_picker.dart';
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/widgets/library_task_button.dart';

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  final StreamController<RequestFailure> _failures =
      StreamController<RequestFailure>.broadcast();
  final scanForces = <bool>[];
  Completer<TaskStatus>? scanCompletion;
  ScanTaskProgress? scanProgress;
  final cleanupConfirmations = <EmptyLibraryCleanupProposal?>[];
  final cleanupResponses = <Completer<EmptyLibraryCleanupResponse>>[];
  final taskStarts = <String>[];
  bool analysisRunning = false;
  // The progress and total each running task reports.
  (int, int) analysisCounts = (25, 100);
  (int, int)? deduplicationCounts;

  @override
  Stream<RequestFailure> get failureFeedback => _failures.stream;

  @override
  ScanTaskProgress? getScanTaskProgress(String? recordId) => scanProgress;

  @override
  Future<EmptyLibraryCleanupResponse> requestEmptyLibraryCleanup(
    String recordId, {
    EmptyLibraryCleanupProposal? confirmation,
  }) {
    cleanupConfirmations.add(confirmation);
    final response = Completer<EmptyLibraryCleanupResponse>();
    cleanupResponses.add(response);
    return response.future;
  }

  @override
  AnalyzeTaskProgress? getAnalyzeTaskProgress(String recordId) =>
      analysisRunning
      ? AnalyzeTaskProgress(
          recordId: recordId,
          progress: analysisCounts.$1,
          total: analysisCounts.$2,
        )
      : null;

  @override
  DeduplicateTaskProgress? getDeduplicateTaskProgress(String recordId) =>
      switch (deduplicationCounts) {
        (final progress, final total) => DeduplicateTaskProgress(
          recordId: recordId,
          progress: progress,
          total: total,
        ),
        null => null,
      };

  @override
  Future<void> scanLibrary(
    String recordId, {
    bool isInitializeTask = false,
    bool force = false,
    Uint64? callId,
  }) async {
    scanForces.add(force);
  }

  @override
  Future<TaskStatus> waitForScanToComplete(String recordId) =>
      scanCompletion?.future ?? Future.value(TaskStatus.finished);

  @override
  Future<void> analyzeLibrary(
    String recordId, [
    bool initialize = false,
    Uint64? callId,
  ]) async {
    taskStarts.add('Analyze');
  }

  @override
  Future<void> waitForAnalyzeToComplete(String recordId) async {}

  @override
  Future<void> deduplicateLibrary(String recordId, {Uint64? callId}) async {
    taskStarts.add('Deduplicate');
  }

  @override
  Future<void> waitForDeduplicateToComplete(String recordId) async {}

  @override
  dynamic noSuchMethod(Invocation invocation) => null;

  @override
  void dispose() {
    _failures.close();
    super.dispose();
  }
}

Future<(LibraryOpenStateProvider, LibraryRegistryProvider)> _openState(
  LibraryRecordSummary record,
) async {
  final registry = LibraryRegistryProvider(
    loadRecords: () async => ListLibraryRecordsResponse(
      records: [record],
      invalidRecords: const [],
      open: OpenRecord(
        recordId: record.recordId,
        generation: Uint64.fromBigInt(BigInt.one),
      ),
      lastOpenRecord: null,
    ),
  );
  final state = LibraryOpenStateProvider(null, registry);
  await state.initialOpen;
  return (state, registry);
}

const _replica = LibraryRecordSummary(
  shareSync: false,
  recordId: 'replica-record',
  alias: 'Replica',
  kind: LibraryRecordKindSummaryReplica(
    libraryId: 'replica-library',
    ownerFingerprint: 'owner',
  ),
  syncAccess: LibraryAccessSummary.read,
);

const _local = LibraryRecordSummary(
  shareSync: false,
  recordId: 'local-record',
  alias: 'Local',
  kind: LibraryRecordKindSummaryLocal(libraryId: 'library', path: '/library'),
);

Future<_LibraryManager> _pumpTaskButtons(
  WidgetTester tester, {
  LibraryRecordSummary record = _replica,
  VoidCallback? onScanFinished,
  bool analysisRunning = false,
  (int, int) analysisCounts = (25, 100),
  (int, int)? deduplicationCounts,
  Future<String?> Function(BuildContext)? pickDirectory,
}) async {
  final (openState, registry) = await _openState(record);
  final manager = _LibraryManager()
    ..analysisRunning = analysisRunning
    ..analysisCounts = analysisCounts
    ..deduplicationCounts = deduplicationCounts;
  addTearDown(openState.dispose);
  addTearDown(registry.dispose);
  addTearDown(manager.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryOpenStateProvider>.value(
          value: openState,
        ),
        ChangeNotifierProvider<LibraryManagerProvider>.value(value: manager),
        Provider(
          create: (_) =>
              LocalFolderPickerProvider(pickDirectory: pickDirectory),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Column(
          children: [
            ScanLibraryButton(title: 'Scan', onFinished: onScanFinished),
            const AnalyzeLibraryButton(title: 'Analyze'),
            const DeduplicateLibraryButton(title: 'Deduplicate'),
          ],
        ),
      ),
    ),
  );
  await tester.pump();
  return manager;
}

void main() {
  const folder = 'content://documents/tree/music';
  const safRecord = LibraryRecordSummary(
    shareSync: false,
    recordId: 'android-library',
    alias: 'Music',
    kind: LibraryRecordKindSummaryLocal(libraryId: 'library', path: folder),
  );
  for (final (selected, retry) in [
    (null, false),
    ('content://documents/tree/other', false),
    ('content://elsewhere/tree/music', false),
    ('$folder/document/child', false),
    ('$folder/document/music', true),
    (folder, true),
  ]) {
    testWidgets(
      'a failed document scan retries only the original folder ($selected)',
      (tester) async {
        var picks = 0;
        final manager = await _pumpTaskButtons(
          tester,
          record: safRecord,
          pickDirectory: (_) async {
            picks++;
            return selected;
          },
        );
        manager.scanProgress = ScanTaskProgress(recordId: safRecord.recordId)
          ..scanFailure = ScanFailure.documentFolderUnavailable;
        final firstScan = Completer<TaskStatus>();
        manager.scanCompletion = firstScan;
        await tester.tap(find.text('Scan'));
        firstScan.complete(TaskStatus.failed);
        await tester.pumpAndSettle();
        manager.scanCompletion = null;
        await tester.tap(find.text('Pick Directory'));
        await tester.pumpAndSettle();
        expect(picks, 1);
        expect(manager.scanForces, retry ? [false, false] : [false]);
        expect(manager.cleanupConfirmations, isEmpty);
        if (selected != null && !retry) {
          expect(
            find.textContaining('You selected a different folder'),
            findsOneWidget,
          );
        }
      },
    );
  }

  for (final confirmationTimedOut in [false, true]) {
    testWidgets(
      confirmationTimedOut
          ? 'cleanup confirmation timeout reports an unknown result without retrying'
          : 'cleanup preview timeout displays the original scan failure',
      (tester) async {
        final manager = await _pumpTaskButtons(tester, record: _local);
        const scanError = 'The scan could not finish.';
        manager.scanProgress = ScanTaskProgress(
          recordId: _local.recordId,
          status: TaskStatus.failed,
        )..error = scanError;
        manager.scanCompletion = Completer<TaskStatus>();
        await tester.tap(find.text('Scan'));
        manager.scanCompletion!.complete(TaskStatus.failed);
        await tester.pumpAndSettle();
        expect(manager.cleanupConfirmations, [null]);

        if (confirmationTimedOut) {
          final proposal = EmptyLibraryCleanupProposal(
            generation: Uint64.fromBigInt(BigInt.one),
            scanRunId: 'scan',
            count: 1,
            fingerprint: 'fingerprint',
          );
          manager.cleanupResponses.single.complete(
            EmptyLibraryCleanupResponse(proposal: proposal, removed: false),
          );
          await tester.pumpAndSettle();
          await tester.tap(find.text('Remove Missing Tracks'));
          await tester.pumpAndSettle();
          expect(manager.cleanupConfirmations, [null, proposal]);
        }

        manager.cleanupResponses.last.completeError(
          TimeoutException('timeout'),
        );
        await tester.pumpAndSettle();
        expect(
          find.text(
            confirmationTimedOut
                ? 'Rune did not receive the cleanup result in time. Check your library before trying again.'
                : scanError,
          ),
          findsOneWidget,
        );
        await tester.tap(find.text('Close'));
        await tester.pumpAndSettle();
        expect(
          manager.cleanupConfirmations,
          hasLength(confirmationTimedOut ? 2 : 1),
        );
      },
    );
  }

  for (final force in [false, true]) {
    testWidgets(
      '${force ? 'force' : 'normal'} scan refreshes only its mounted caller',
      (tester) async {
        var refreshes = 0;
        final manager = await _pumpTaskButtons(
          tester,
          onScanFinished: () => refreshes++,
        );
        final buttons = force ? kSecondaryMouseButton : kPrimaryMouseButton;
        final scan = find.text('Scan');
        manager.scanCompletion = Completer<TaskStatus>();
        await tester.tap(scan, buttons: buttons);
        expect(manager.scanForces, [force]);
        expect(refreshes, 0);
        manager.scanCompletion!.complete(TaskStatus.finished);
        await tester.pump();
        expect(refreshes, 1);

        manager.scanCompletion = Completer<TaskStatus>();
        await tester.tap(scan, buttons: buttons);
        expect(manager.scanForces, [force, force]);
        await tester.pumpAndSettle();
        await tester.pumpWidget(const SizedBox());
        manager.scanCompletion!.complete(TaskStatus.finished);
        await tester.pump();
        expect(refreshes, 1, reason: 'the scan caller has unmounted');
      },
    );
  }

  // A task's first update reports 0 of 0, before it has counted its work.
  for (final task in ['Analyze', 'Deduplicate']) {
    testWidgets('$task shows an indeterminate ring before its total is known', (
      tester,
    ) async {
      await _pumpTaskButtons(
        tester,
        analysisRunning: task == 'Analyze',
        analysisCounts: (0, 0),
        deduplicationCounts: task == 'Deduplicate' ? (0, 0) : null,
      );

      expect(tester.takeException(), isNull);
      expect(
        tester.widget<ProgressRing>(find.byType(ProgressRing)).value,
        isNull,
      );
    });
  }

  testWidgets('a running analysis shows the share it has done', (tester) async {
    await _pumpTaskButtons(tester, analysisRunning: true);

    expect(tester.widget<ProgressRing>(find.byType(ProgressRing)).value, 25);
  });

  for (final task in ['Analyze', 'Deduplicate']) {
    testWidgets('touch long press on $task does not start a task', (
      tester,
    ) async {
      final manager = await _pumpTaskButtons(tester);
      final button = find.text(task);

      await tester.longPress(button);
      expect(tester.takeException(), isNull);
      expect(manager.taskStarts, isEmpty);

      await tester.tap(button);
      await tester.pumpAndSettle();
      expect(manager.taskStarts, [task]);
    }, variant: TargetPlatformVariant.only(TargetPlatform.android));
  }

  testWidgets(
    'touch long press on Scan forces one rescan and a tap starts a normal scan',
    (tester) async {
      final manager = await _pumpTaskButtons(tester);
      final scan = find.text('Scan');

      await tester.longPress(scan);
      expect(tester.takeException(), isNull);
      expect(manager.scanForces, [true]);

      await tester.tap(scan);
      await tester.pumpAndSettle();
      expect(manager.scanForces, [true, false]);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  testWidgets('Scan ignores alternate actions and taps while analysis runs', (
    tester,
  ) async {
    final manager = await _pumpTaskButtons(tester, analysisRunning: true);
    final scan = find.text('Scan');

    await tester.tap(scan, buttons: kSecondaryMouseButton);
    await tester.longPress(scan);
    await tester.tap(scan);
    await tester.pump();

    expect(tester.takeException(), isNull);
    expect(manager.scanForces, isEmpty);
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));

  testWidgets('catalog task buttons appear for replicas, not remote control', (
    tester,
  ) async {
    await _pumpTaskButtons(tester);
    for (final task in ['Scan', 'Analyze', 'Deduplicate']) {
      expect(find.text(task), findsOneWidget);
    }

    await tester.pumpWidget(const SizedBox());
    await _pumpTaskButtons(
      tester,
      record: const LibraryRecordSummary(
        shareSync: false,
        recordId: 'remote-record',
        alias: 'Remote control',
        kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
      ),
    );
    for (final task in ['Scan', 'Analyze', 'Deduplicate']) {
      expect(find.text(task), findsNothing);
    }
  });

  for (final service in OpenDalServiceKind.values) {
    testWidgets(
      'catalog task buttons start tasks for a ${service.name} library',
      (tester) async {
        final record = LibraryRecordSummary(
          shareSync: false,
          recordId: '${service.name}-record',
          alias: 'music',
          kind: LibraryRecordKindSummaryOpenDal(
            libraryId: '${service.name}-library',
            service: service,
            endpoint: 'https://storage.example',
            root: 'music',
          ),
        );
        final manager = await _pumpTaskButtons(tester, record: record);
        await tester.tap(find.text('Scan'));
        await tester.pumpAndSettle();
        expect(manager.scanForces, [false]);
        await tester.tap(find.text('Analyze'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Deduplicate'));
        await tester.pumpAndSettle();
        expect(manager.taskStarts, ['Analyze', 'Deduplicate']);

        final (openState, registry) = await _openState(record);
        addTearDown(openState.dispose);
        addTearDown(registry.dispose);
        expect(openState.catalogLibraryId, '${service.name}-library');
      },
    );
  }
}
