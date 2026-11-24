import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

void main() {
  test(
    'library task progress reflects the full state reported by the node',
    () async {
      final states = StreamController<LibraryTaskStateChanged>();
      final manager = LibraryManagerProvider(libraryTaskStates: states.stream);
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateAnalysisRunning(
            recordId: 'library',
            runId: 'run-1',
            progress: 2,
            total: 5,
          ),
        ),
      );
      await pumpEventQueue();
      expect(manager.getAnalyzeTaskProgress('library')?.progress, 2);
      expect(manager.getAnalyzeTaskProgress('library')?.total, 5);
      expect(
        manager.getAnalyzeTaskProgress('library')?.status,
        TaskStatus.working,
      );

      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateAnalysisEnded(
            recordId: 'library',
            runId: 'run-1',
            progress: 2,
            total: 5,
            outcome: AnalysisOutcomeCancelled(),
          ),
        ),
      );
      await pumpEventQueue();
      expect(
        manager.getAnalyzeTaskProgress('library')?.status,
        TaskStatus.cancelled,
      );

      manager.dispose();
      await states.close();
    },
  );

  test(
    'a late cancellation of an old run leaves the restarted run working',
    () async {
      final states = StreamController<LibraryTaskStateChanged>.broadcast(
        sync: true,
      );
      final manager = LibraryManagerProvider(libraryTaskStates: states.stream);
      addTearDown(states.close);
      addTearDown(manager.dispose);
      const recordId = 'library';

      const oldRunning = LibraryTaskStateChanged(
        state: LibraryTaskStateScanRunning(
          recordId: recordId,
          runId: 'old-run',
          progress: 1,
          total: 10,
          phase: ScanTaskPhase.indexFiles,
        ),
      );
      const oldCancelled = LibraryTaskStateChanged(
        state: LibraryTaskStateScanEnded(
          recordId: recordId,
          runId: 'old-run',
          progress: 1,
          total: 10,
          phase: ScanTaskPhase.indexFiles,
          outcome: ScanOutcomeCancelled(),
        ),
      );

      states.add(oldRunning);
      expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.working);
      states.add(oldCancelled);
      expect(
        manager.getScanTaskProgress(recordId)?.status,
        TaskStatus.cancelled,
      );

      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanRunning(
            recordId: recordId,
            runId: 'new-run',
            progress: 0,
            total: 0,
            phase: ScanTaskPhase.indexFiles,
          ),
        ),
      );
      expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.working);

      var waiterCompleted = false;
      final waiter = manager.waitForScanToComplete(recordId);
      unawaited(waiter.then((_) => waiterCompleted = true));
      await pumpEventQueue();
      expect(waiterCompleted, isFalse);

      // The old run's cancellation is delivered after the restart.
      states.add(oldCancelled);
      await pumpEventQueue();
      expect(manager.getScanTaskProgress(recordId)?.status, TaskStatus.working);
      expect(waiterCompleted, isFalse);

      // The restarted run's own end still updates the display and waiter.
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanEnded(
            recordId: recordId,
            runId: 'new-run',
            progress: 10,
            total: 10,
            phase: ScanTaskPhase.indexFiles,
            outcome: ScanOutcomeFinished(),
          ),
        ),
      );
      await pumpEventQueue();
      expect(
        manager.getScanTaskProgress(recordId)?.status,
        TaskStatus.finished,
      );
      expect(await waiter, TaskStatus.finished);
      expect(waiterCompleted, isTrue);
    },
  );

  test(
    'every task kind maps its own variants to the shared task status',
    () async {
      final states = StreamController<LibraryTaskStateChanged>();
      final manager = LibraryManagerProvider(libraryTaskStates: states.stream);

      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateAnalysisEnded(
            recordId: 'library',
            runId: 'run-1',
            progress: 0,
            total: 0,
            outcome: AnalysisOutcomeFailed(
              value: AnalysisFailure.recommendationSyncFailed,
            ),
          ),
        ),
      );
      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateDeduplicationRunning(
            recordId: 'library',
            runId: 'run-2',
            progress: 40,
            total: 100,
          ),
        ),
      );
      await pumpEventQueue();
      expect(
        manager.getAnalyzeTaskProgress('library')?.status,
        TaskStatus.failed,
      );
      expect(manager.getDeduplicateTaskProgress('library')?.progress, 40);
      expect(
        manager.getDeduplicateTaskProgress('library')?.status,
        TaskStatus.working,
      );

      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateDeduplicationEnded(
            recordId: 'library',
            runId: 'run-2',
            progress: 0,
            total: 100,
            outcome: DeduplicationOutcomeFailed(
              value: DeduplicationFailure.comparisonFailed,
            ),
          ),
        ),
      );
      await pumpEventQueue();
      expect(
        manager.getDeduplicateTaskProgress('library')?.status,
        TaskStatus.failed,
      );

      states.add(
        const LibraryTaskStateChanged(
          state: LibraryTaskStateScanEnded(
            recordId: 'library',
            runId: 'run-3',
            progress: 2,
            total: 9,
            phase: ScanTaskPhase.scanCoverArts,
            outcome: ScanOutcomeFailed(value: ScanFailure.workerFailed),
          ),
        ),
      );
      await pumpEventQueue();
      final scan = manager.getScanTaskProgress('library');
      expect(scan?.status, TaskStatus.failed);
      expect(scan?.phase, ScanTaskPhase.scanCoverArts);
      expect(scan?.error, 'Scan failed');

      manager.dispose();
      await states.close();
    },
  );

  test(
    'library start failures are exposed with their typed running task',
    () async {
      final states = StreamController<LibraryTaskStateChanged>();
      final failures = StreamController<RequestFailedResponse>();
      final requests = <ScanAudioLibraryRequest>[];
      final manager = LibraryManagerProvider(
        sendScanRequest: requests.add,
        libraryTaskStates: states.stream,
        requestFailures: failures.stream,
      );
      final callId = Uint64.fromBigInt(BigInt.from(17));
      await manager.scanLibrary('library', callId: callId);
      final feedback = manager.failureFeedback.first;
      failures.add(
        RequestFailedResponse(
          request: 'diagnostic only',
          callId: requests.single.callId,
          kind: RequestErrorKind.conflict,
          detail: 'Cannot start a library task while scan is running',
          cause: RequestFailureCauseLibraryTaskAlreadyRunning(
            task: LibraryTaskKind.scan,
          ),
        ),
      );
      final failure = await feedback;
      expect(failure.callId, callId.toInt());
      expect(failure.kind, RequestErrorKind.conflict);
      expect(
        failure.cause,
        const RequestFailureCauseLibraryTaskAlreadyRunning(
          task: LibraryTaskKind.scan,
        ),
      );

      manager.dispose();
      await states.close();
      await failures.close();
    },
  );
}
