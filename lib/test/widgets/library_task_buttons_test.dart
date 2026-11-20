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
import 'package:rune/utils/api/request_failure.dart';
import 'package:rune/widgets/library_task_button.dart';

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  final StreamController<RequestFailure> _failures =
      StreamController<RequestFailure>.broadcast();
  final scanForces = <bool>[];
  final taskStarts = <String>[];
  bool analysisRunning = false;
  // The progress and total each running task reports.
  (int, int) analysisCounts = (25, 100);
  (int, int)? deduplicationCounts;

  @override
  Stream<RequestFailure> get failureFeedback => _failures.stream;

  @override
  ScanTaskProgress? getScanTaskProgress(String? recordId) => null;

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
  Future<TaskStatus> waitForScanToComplete(String recordId) async =>
      TaskStatus.finished;

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
  recordId: 'replica-record',
  alias: 'Replica',
  kind: LibraryRecordKindSummaryReplica(
    libraryId: 'replica-library',
    ownerFingerprint: 'owner',
  ),
  syncAccess: LibraryAccessSummary.read,
);

Future<_LibraryManager> _pumpTaskButtons(
  WidgetTester tester, {
  LibraryRecordSummary record = _replica,
  bool analysisRunning = false,
  (int, int) analysisCounts = (25, 100),
  (int, int)? deduplicationCounts,
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
      ],
      child: const FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Column(
          children: [
            ScanLibraryButton(title: 'Scan'),
            AnalyzeLibraryButton(title: 'Analyze'),
            DeduplicateLibraryButton(title: 'Deduplicate'),
          ],
        ),
      ),
    ),
  );
  await tester.pump();
  return manager;
}

void main() {
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
        recordId: 'remote-record',
        alias: 'Remote control',
        kind: LibraryRecordKindSummaryRemoteControl(fingerprint: 'renderer'),
      ),
    );
    for (final task in ['Scan', 'Analyze', 'Deduplicate']) {
      expect(find.text(task), findsNothing);
    }
  });
}
