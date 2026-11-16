import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/api/request_failure.dart';
import '../utils/api/call_id.dart';

enum TaskStatus { working, finished, cancelled, failed }

class _PendingLibraryTaskCall {
  const _PendingLibraryTaskCall({
    required this.recordId,
    required this.task,
    required this.fromButton,
  });

  final String recordId;
  final LibraryTaskKind task;
  final bool fromButton;
}

class AnalyzeTaskProgress {
  final String recordId;
  int progress;
  int total;
  TaskStatus status;
  bool isInitializeTask;

  AnalyzeTaskProgress({
    required this.recordId,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
    this.isInitializeTask = false,
  });

  @override
  String toString() {
    return 'AnalyzeTaskProgress(recordId: $recordId, progress: $progress, total: $total, status: $status, initialize: $isInitializeTask)';
  }
}

class ScanTaskProgress {
  final String recordId;
  int progress;
  int total;
  ScanTaskPhase phase;
  TaskStatus status;
  bool initialize;
  String? error;
  RequestFailure? failure;

  ScanTaskProgress({
    required this.recordId,
    this.progress = 0,
    this.total = 0,
    this.phase = ScanTaskPhase.indexFiles,
    this.status = TaskStatus.working,
    this.initialize = false,
  });

  @override
  String toString() {
    return 'ScanTaskProgress(recordId: $recordId, progress: $progress, total: $total, status: $status, initialize: $initialize)';
  }
}

class DeduplicateTaskProgress {
  final String recordId;
  int progress;
  int total;
  TaskStatus status;

  DeduplicateTaskProgress({
    required this.recordId,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
  });

  @override
  String toString() {
    return 'DeduplicateTaskProgress(recordId: $recordId, progress: $progress, total: $total, status: $status)';
  }
}

class LibraryManagerProvider with ChangeNotifier {
  final Map<String, AnalyzeTaskProgress> _analyzeTasks = {};
  final Map<String, ScanTaskProgress> _scanTasks = {};
  final Map<String, DeduplicateTaskProgress> _deduplicateTasks = {};

  StreamSubscription? _libraryContentSubscription;
  StreamSubscription? _libraryTaskSubscription;
  StreamSubscription? _requestFailureSubscription;
  final StreamController<RequestFailure> _failureFeedback =
      StreamController<RequestFailure>.broadcast();

  final Map<String, Completer<TaskStatus>> _scanCompleters = {};
  final Map<String, Completer<void>> _analyzeCompleters = {};
  final Map<String, Completer<void>> _deduplicateCompleters = {};
  final Map<String, bool> _analysisInitializeIntent = {};
  final Map<String, bool> _scanInitializeUsesLibraryNavigator = {};
  final Map<int, _PendingLibraryTaskCall> _pendingTaskCalls = {};

  final void Function(ScanAudioLibraryRequest)? _sendScanRequest;
  final void Function(String)? _navigate;
  final Stream<LibraryTaskStateChanged> _libraryTaskStates;
  final Stream<RequestFailedResponse> _requestFailures;

  LibraryManagerProvider({
    void Function(ScanAudioLibraryRequest)? sendScanRequest,
    void Function(String)? navigate,
    Stream<LibraryTaskStateChanged>? libraryTaskStates,
    Stream<RequestFailedResponse>? requestFailures,
  }) : _sendScanRequest = sendScanRequest,
       _navigate = navigate,
       _libraryTaskStates =
           libraryTaskStates ??
           LibraryTaskStateChanged.rustSignalStream.map(
             (event) => event.message,
           ),
       _requestFailures =
           requestFailures ??
           RequestFailedResponse.rustSignalStream.map(
             (event) => event.message,
           ) {
    initListeners();
  }

  void initListeners() {
    _libraryContentSubscription = LibraryContentChanged.rustSignalStream.listen(
      (_) {
        CollectionCache().clearAll();
      },
    );
    _libraryTaskSubscription = _libraryTaskStates.listen((event) {
      _applyLibraryTaskState(event.state);
    });
    _requestFailureSubscription = _requestFailures.listen(
      _handleRequestFailure,
    );
  }

  void _handleRequestFailure(RequestFailedResponse response) {
    final id = response.callId?.toInt();
    if (id == null) return;
    final pending = _pendingTaskCalls.remove(id);
    if (pending == null) return;
    final failure = RequestFailure(
      response.request,
      response.kind,
      response.detail,
      response.cause,
      id,
    );
    if (pending.fromButton) {
      _failureFeedback.add(failure);
      return;
    }
    switch (pending.task) {
      case LibraryTaskKind.scan:
        _updateScanProgress(
          pending.recordId,
          0,
          0,
          ScanTaskPhase.indexFiles,
          TaskStatus.failed,
          _scanTasks[pending.recordId]?.initialize ?? false,
        );
        _scanTasks[pending.recordId]!
          ..failure = failure
          ..error = failure.detail;
        _scanCompleters.remove(pending.recordId)?.complete(TaskStatus.failed);
      case LibraryTaskKind.analyze:
        _updateAnalyzeProgress(
          pending.recordId,
          0,
          0,
          TaskStatus.failed,
          _analysisInitializeIntent.remove(pending.recordId) ?? false,
        );
        _analyzeCompleters.remove(pending.recordId)?.complete();
      case LibraryTaskKind.deduplicate:
        _updateDeduplicateProgress(pending.recordId, 0, 0, TaskStatus.failed);
        _deduplicateCompleters.remove(pending.recordId)?.complete();
    }
  }

  void _forgetTaskCalls(String recordId, LibraryTaskKind task) {
    _pendingTaskCalls.removeWhere(
      (_, pending) => pending.recordId == recordId && pending.task == task,
    );
  }

  Stream<RequestFailure> get failureFeedback => _failureFeedback.stream;

  void _applyLibraryTaskState(LibraryTaskState state) {
    switch (state) {
      case LibraryTaskStateNotRunning():
        return;
      case LibraryTaskStateScanRunning(
        :final recordId,
        :final progress,
        :final total,
        :final phase,
      ):
        _updateScanProgress(
          recordId,
          progress,
          total,
          phase,
          TaskStatus.working,
          getScanTaskProgress(recordId)?.initialize ?? false,
        );
        CollectionCache().clearAll();
      case LibraryTaskStateScanEnded(
        :final recordId,
        :final progress,
        :final total,
        :final phase,
        :final outcome,
      ):
        _forgetTaskCalls(recordId, LibraryTaskKind.scan);
        final status = switch (outcome) {
          ScanOutcomeFinished() => TaskStatus.finished,
          ScanOutcomeCancelled() => TaskStatus.cancelled,
          _ => TaskStatus.failed,
        };
        _updateScanProgress(
          recordId,
          progress,
          total,
          phase,
          status,
          getScanTaskProgress(recordId)?.initialize ?? false,
        );
        if (outcome is ScanOutcomeFailed) {
          getScanTaskProgress(recordId)?.error = 'Scan failed';
        }
        if (status == TaskStatus.finished &&
            getScanTaskProgress(recordId)?.initialize == true) {
          final nestedNavigator = _scanInitializeUsesLibraryNavigator.remove(
            recordId,
          );
          if (nestedNavigator != null) {
            if (_navigate case final navigate?) {
              navigate('/library');
            } else if (nestedNavigator) {
              $replace('/library');
            } else {
              $$replace('/library');
            }
          }
        }
        _scanCompleters.remove(recordId)?.complete(status);
        CollectionCache().clearAll();
      case LibraryTaskStateAnalysisRunning(
        :final recordId,
        :final progress,
        :final total,
      ):
        _updateAnalyzeProgress(
          recordId,
          progress,
          total,
          TaskStatus.working,
          getAnalyzeTaskProgress(recordId)?.isInitializeTask ??
              _analysisInitializeIntent[recordId] ??
              false,
        );
      case LibraryTaskStateAnalysisEnded(
        :final recordId,
        :final progress,
        :final total,
        :final outcome,
      ):
        _forgetTaskCalls(recordId, LibraryTaskKind.analyze);
        final status = switch (outcome) {
          AnalysisOutcomeFinished() => TaskStatus.finished,
          AnalysisOutcomeCancelled() => TaskStatus.cancelled,
          _ => TaskStatus.failed,
        };
        _updateAnalyzeProgress(
          recordId,
          progress,
          total,
          status,
          getAnalyzeTaskProgress(recordId)?.isInitializeTask ??
              _analysisInitializeIntent.remove(recordId) ??
              false,
        );
        _analyzeCompleters.remove(recordId)?.complete();
      case LibraryTaskStateDeduplicationRunning(
        :final recordId,
        :final progress,
        :final total,
      ):
        _updateDeduplicateProgress(
          recordId,
          progress,
          total,
          TaskStatus.working,
        );
      case LibraryTaskStateDeduplicationEnded(
        :final recordId,
        :final progress,
        :final total,
        :final outcome,
      ):
        _forgetTaskCalls(recordId, LibraryTaskKind.deduplicate);
        final status = switch (outcome) {
          DeduplicationOutcomeFinished() => TaskStatus.finished,
          DeduplicationOutcomeCancelled() => TaskStatus.cancelled,
          _ => TaskStatus.failed,
        };
        _updateDeduplicateProgress(recordId, progress, total, status);
        _deduplicateCompleters.remove(recordId)?.complete();
    }
  }

  void _updateScanProgress(
    String recordId,
    int progress,
    int total,
    ScanTaskPhase phase,
    TaskStatus status,
    bool initialize,
  ) {
    if (_scanTasks.containsKey(recordId)) {
      _scanTasks[recordId]!.progress = progress;
      _scanTasks[recordId]!.total = total;
      _scanTasks[recordId]!.phase = phase;
      _scanTasks[recordId]!.status = status;
      _scanTasks[recordId]!.initialize = initialize;
      if (status == TaskStatus.working) {
        _scanTasks[recordId]!
          ..error = null
          ..failure = null;
      }
    } else {
      _scanTasks[recordId] = ScanTaskProgress(
        recordId: recordId,
        progress: progress,
        total: total,
        phase: phase,
        status: status,
        initialize: initialize,
      );
    }
    notifyListeners();
  }

  void _updateAnalyzeProgress(
    String recordId,
    int progress,
    int total,
    TaskStatus status,
    bool initialize,
  ) {
    if (_analyzeTasks.containsKey(recordId)) {
      _analyzeTasks[recordId]!.progress = progress;
      _analyzeTasks[recordId]!.total = total;
      _analyzeTasks[recordId]!.status = status;
    } else {
      _analyzeTasks[recordId] = AnalyzeTaskProgress(
        recordId: recordId,
        progress: progress,
        total: total,
        status: status,
        isInitializeTask: initialize,
      );
    }
    notifyListeners();
  }

  void _updateDeduplicateProgress(
    String recordId,
    int progress,
    int total,
    TaskStatus status,
  ) {
    if (_deduplicateTasks.containsKey(recordId)) {
      _deduplicateTasks[recordId]!.progress = progress;
      _deduplicateTasks[recordId]!.total = total;
      _deduplicateTasks[recordId]!.status = status;
    } else {
      _deduplicateTasks[recordId] = DeduplicateTaskProgress(
        recordId: recordId,
        progress: progress,
        total: total,
        status: status,
      );
    }
    notifyListeners();
  }

  void clearAll(String recordId) {
    _pendingTaskCalls.removeWhere((_, pending) => pending.recordId == recordId);
    _settleScanWaiters();
    _scanTasks.clear();
    _scanInitializeUsesLibraryNavigator.clear();
    _analyzeTasks.clear();
    _deduplicateTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(
    String recordId, {
    bool isInitializeTask = false,
    bool force = false,
    Uint64? callId,
  }) async {
    final requestCallId = callId ?? nextCallId();
    _pendingTaskCalls[requestCallId.toInt()] = _PendingLibraryTaskCall(
      recordId: recordId,
      task: LibraryTaskKind.scan,
      fromButton: callId != null,
    );
    if (isInitializeTask) {
      _scanInitializeUsesLibraryNavigator[recordId] = $hasLibraryNavigator;
      if (_navigate case final navigate?) {
        navigate('/scanning');
      } else if ($hasLibraryNavigator) {
        $replace('/scanning');
      } else {
        $$replace('/scanning');
      }
    }
    _scanCompleters.putIfAbsent(recordId, Completer<TaskStatus>.new);
    _updateScanProgress(
      recordId,
      0,
      0,
      ScanTaskPhase.indexFiles,
      TaskStatus.working,
      isInitializeTask,
    );
    final request = ScanAudioLibraryRequest(
      force: force,
      callId: requestCallId,
    );
    if (_sendScanRequest case final send?) {
      send(request);
    } else {
      request.sendSignalToRust();
    }
  }

  Future<void> analyzeLibrary(
    String recordId, [
    bool initialize = false,
    Uint64? callId,
  ]) async {
    final requestCallId = callId ?? nextCallId();
    _pendingTaskCalls[requestCallId.toInt()] = _PendingLibraryTaskCall(
      recordId: recordId,
      task: LibraryTaskKind.analyze,
      fromButton: callId != null,
    );
    _analysisInitializeIntent[recordId] = initialize;
    _analyzeCompleters.putIfAbsent(recordId, Completer<void>.new);
    final computingDevice = 'cpu';

    double workloadFactor = 0.75;

    String? performanceLevel = await $settingsManager.getValue<String>(
      kAnalysisPerformanceLevelKey,
    );

    if (performanceLevel == "balance") {
      workloadFactor = 0.5;
    }

    if (performanceLevel == "battery") {
      workloadFactor = 0.25;
    }

    AnalyzeAudioLibraryRequest(
      computingDevice: computingDevice == 'gpu'
          ? ComputingDeviceRequest.gpu
          : ComputingDeviceRequest.cpu,
      workloadFactor: workloadFactor,
      callId: requestCallId,
    ).sendSignalToRust();
  }

  Future<void> deduplicateLibrary(String recordId, {Uint64? callId}) async {
    final requestCallId = callId ?? nextCallId();
    _pendingTaskCalls[requestCallId.toInt()] = _PendingLibraryTaskCall(
      recordId: recordId,
      task: LibraryTaskKind.deduplicate,
      fromButton: callId != null,
    );
    _deduplicateCompleters.putIfAbsent(recordId, Completer<void>.new);
    double workloadFactor = 0.75;
    String similarityLevel =
        await $settingsManager.getValue<String>(kDeduplicateThresholdKey) ??
        "0.85";
    double similarityThreshold = double.tryParse(similarityLevel) ?? 0.85;

    DeduplicateAudioLibraryRequest(
      similarityThreshold: similarityThreshold,
      workloadFactor: workloadFactor,
      callId: requestCallId,
    ).sendSignalToRust();
  }

  ScanTaskProgress? getScanTaskProgress(String? recordId) {
    return _scanTasks[recordId];
  }

  AnalyzeTaskProgress? getAnalyzeTaskProgress(String recordId) {
    return _analyzeTasks[recordId];
  }

  DeduplicateTaskProgress? getDeduplicateTaskProgress(String recordId) {
    return _deduplicateTasks[recordId];
  }

  Future<TaskStatus> waitForScanToComplete(String recordId) {
    final taskProgress = getScanTaskProgress(recordId);
    if (taskProgress == null || taskProgress.status != TaskStatus.working) {
      return Future.value(taskProgress?.status ?? TaskStatus.finished);
    }

    final existed = _scanCompleters[recordId];
    if (existed != null) return existed.future;

    _scanCompleters[recordId] = Completer<TaskStatus>();
    return _scanCompleters[recordId]!.future;
  }

  Future<void> waitForAnalyzeToComplete(String recordId) {
    final taskProgress = getAnalyzeTaskProgress(recordId);
    if (taskProgress != null && taskProgress.status != TaskStatus.working) {
      return Future.value();
    }
    if (taskProgress == null && !_analyzeCompleters.containsKey(recordId)) {
      return Future.value();
    }

    final existed = _analyzeCompleters[recordId];
    if (existed != null) return existed.future;

    _analyzeCompleters[recordId] = Completer<void>();
    return _analyzeCompleters[recordId]!.future;
  }

  Future<void> waitForDeduplicateToComplete(String recordId) {
    final taskProgress = getDeduplicateTaskProgress(recordId);
    if (taskProgress != null && taskProgress.status != TaskStatus.working) {
      return Future.value();
    }
    if (taskProgress == null && !_deduplicateCompleters.containsKey(recordId)) {
      return Future.value();
    }

    final existed = _deduplicateCompleters[recordId];
    if (existed != null) return existed.future;

    _deduplicateCompleters[recordId] = Completer<void>();
    return _deduplicateCompleters[recordId]!.future;
  }

  Future<void> cancelTask(String recordId, CancelTaskType type) async {
    CancelTaskRequest(recordId: recordId, rType: type).sendSignalToRust();
  }

  void _settleScanWaiters() {
    for (final completer in _scanCompleters.values) {
      completer.complete(TaskStatus.cancelled);
    }
    _scanCompleters.clear();
  }

  @override
  void dispose() {
    _settleScanWaiters();
    _libraryContentSubscription?.cancel();
    _libraryTaskSubscription?.cancel();
    _requestFailureSubscription?.cancel();
    _failureFeedback.close();
    super.dispose();
  }
}
