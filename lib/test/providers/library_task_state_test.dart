import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

void main() {
  test('library task progress reflects the full state reported by the node', () async {
    final states = StreamController<LibraryTaskStateChanged>();
    final manager = LibraryManagerProvider(libraryTaskStates: states.stream);
    states.add(
      const LibraryTaskStateChanged(
        state: LibraryTaskStateRunning(
          recordId: 'library',
          task: LibraryTaskKind.analyze,
          runId: 'run-1',
          progress: 2,
          total: 5,
        ),
      ),
    );
    await pumpEventQueue();
    expect(manager.getAnalyzeTaskProgress('library')?.progress, 2);
    expect(manager.getAnalyzeTaskProgress('library')?.total, 5);
    expect(manager.getAnalyzeTaskProgress('library')?.status, TaskStatus.working);

    states.add(
      const LibraryTaskStateChanged(
        state: LibraryTaskStateEnded(
          recordId: 'library',
          task: LibraryTaskKind.analyze,
          runId: 'run-1',
          progress: 2,
          total: 5,
          outcome: LibraryTaskOutcomeCancelled(),
        ),
      ),
    );
    await pumpEventQueue();
    expect(manager.getAnalyzeTaskProgress('library')?.status, TaskStatus.cancelled);

    manager.dispose();
    await states.close();
  });

  test('library start failures are exposed with their typed running task', () async {
    final states = StreamController<LibraryTaskStateChanged>();
    final failures = StreamController<RequestFailedResponse>();
    final manager = LibraryManagerProvider(
      libraryTaskStates: states.stream,
      requestFailures: failures.stream,
    );
    final feedback = manager.failureFeedback.first;
    failures.add(
      const RequestFailedResponse(
        request: 'AnalyzeAudioLibraryRequest',
        kind: RequestErrorKind.conflict,
        detail: 'Cannot start a library task while scan is running',
        cause: RequestFailureCauseLibraryTaskAlreadyRunning(
          task: LibraryTaskKind.scan,
        ),
      ),
    );
    final failure = await feedback;
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
  });
}
