import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';
import '../utils/api/request_failure.dart';

enum TaskStatus { working, finished, cancelled, failed }

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
  ScanTaskType type;
  int progress;
  TaskStatus status;
  bool initialize;
  String? error;

  ScanTaskProgress({
    required this.recordId,
    required this.type,
    this.progress = 0,
    this.status = TaskStatus.working,
    this.initialize = false,
  });

  @override
  String toString() {
    return 'ScanTaskProgress(recordId: $recordId, progress: $progress, status: $status, type: $type, initialize: $initialize)';
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
  StreamSubscription? _scanProgressSubscription;
  StreamSubscription? _scanResultSubscription;
  StreamSubscription? _libraryTaskSubscription;
  StreamSubscription? _requestFailureSubscription;
  final StreamController<RequestFailure> _failureFeedback =
      StreamController<RequestFailure>.broadcast();

  final Map<String, Completer<TaskStatus>> _scanCompleters = {};
  final Map<String, String> _scanIds = {};
  int _nextScanId = 0;
  final Map<String, Completer<void>> _analyzeCompleters = {};
  final Map<String, Completer<void>> _deduplicateCompleters = {};
  final Map<String, bool> _analysisInitializeIntent = {};

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
       _libraryTaskStates = libraryTaskStates ??
           LibraryTaskStateChanged.rustSignalStream.map((event) => event.message),
       _requestFailures = requestFailures ??
           RequestFailedResponse.rustSignalStream.map((event) => event.message) {
    initListeners();
  }

  void initListeners() {
    _libraryContentSubscription = LibraryContentChanged.rustSignalStream.listen(
      (_) {
        CollectionCache().clearAll();
      },
    );
    _scanProgressSubscription = ScanAudioLibraryProgress.rustSignalStream
        .listen((event) {
          final scanProgress = event.message;
          if (_scanIds[scanProgress.recordId] != scanProgress.scanId ||
              getScanTaskProgress(scanProgress.recordId)?.status !=
                  TaskStatus.working) {
            return;
          }
          _updateScanProgress(
            scanProgress.recordId,
            scanProgress.task,
            scanProgress.progress,
            scanProgress.total,
            TaskStatus.working,
            getScanTaskProgress(scanProgress.recordId)?.initialize ?? false,
          );
          CollectionCache().clearAll();
        });

    _scanResultSubscription = ScanAudioLibraryResponse.rustSignalStream.listen((
      event,
    ) {
      final scanResult = event.message;
      if (_scanIds[scanResult.recordId] != scanResult.scanId) return;
      final status = switch (scanResult.status) {
        "success" => TaskStatus.finished,
        "cancelled" => TaskStatus.cancelled,
        _ => TaskStatus.failed,
      };
      final initialize =
          getScanTaskProgress(scanResult.recordId)?.initialize ?? false;
      _updateScanProgress(
        scanResult.recordId,
        ScanTaskType.scanCoverArts,
        scanResult.progress,
        0,
        status,
        initialize,
      );

      getScanTaskProgress(scanResult.recordId)?.error = scanResult.error;
      notifyListeners();
      if (initialize && status == TaskStatus.finished) {
        if (_navigate case final navigate?) {
          navigate("/library");
        } else {
          $$replace("/library");
        }
      }

      // Complete the scan task
      _scanCompleters[scanResult.recordId]?.complete(status);
      _scanCompleters.remove(scanResult.recordId);
      CollectionCache().clearAll();
    });

    _libraryTaskSubscription = _libraryTaskStates.listen((event) {
      _applyLibraryTaskState(event.state);
    });
    _requestFailureSubscription = _requestFailures.listen((failure) {
      if (failure.request == 'ScanAudioLibraryRequest' ||
          failure.request == 'AnalyzeAudioLibraryRequest' ||
          failure.request == 'DeduplicateAudioLibraryRequest') {
        _failureFeedback.add(
          RequestFailure(
            failure.request,
            failure.kind,
            failure.detail,
            failure.cause,
          ),
        );
      }
    });
  }

  Stream<RequestFailure> get failureFeedback => _failureFeedback.stream;

  void _applyLibraryTaskState(LibraryTaskState state) {
    if (state case LibraryTaskStateRunning(:final recordId, :final task, :final progress, :final total)) {
      switch (task) {
        case LibraryTaskKind.analyze:
          _updateAnalyzeProgress(recordId, progress, total, TaskStatus.working,
            getAnalyzeTaskProgress(recordId)?.isInitializeTask ??
                _analysisInitializeIntent[recordId] ?? false);
        case LibraryTaskKind.deduplicate:
          _updateDeduplicateProgress(recordId, progress, total, TaskStatus.working);
        case LibraryTaskKind.scan:
          return;
      }
      return;
    }
    if (state case LibraryTaskStateEnded(:final recordId, :final task, :final progress, :final total, :final outcome)) {
      final status = switch (outcome) {
        LibraryTaskOutcomeFinished() => TaskStatus.finished,
        LibraryTaskOutcomeCancelled() => TaskStatus.cancelled,
        _ => TaskStatus.failed,
      };
      switch (task) {
        case LibraryTaskKind.analyze:
          _updateAnalyzeProgress(recordId, progress, total, status,
            getAnalyzeTaskProgress(recordId)?.isInitializeTask ??
                _analysisInitializeIntent.remove(recordId) ?? false);
          _analyzeCompleters.remove(recordId)?.complete();
        case LibraryTaskKind.deduplicate:
          _updateDeduplicateProgress(recordId, progress, total, status);
          _deduplicateCompleters.remove(recordId)?.complete();
        case LibraryTaskKind.scan:
          return;
      }
    }
  }

  void _updateScanProgress(
    String recordId,
    ScanTaskType taskType,
    int progress,
    int total,
    TaskStatus status,
    bool initialize,
  ) {
    if (_scanTasks.containsKey(recordId)) {
      _scanTasks[recordId]!.progress = progress;
      _scanTasks[recordId]!.status = status;
      _scanTasks[recordId]!.type = taskType;
      _scanTasks[recordId]!.initialize = initialize;
      if (status == TaskStatus.working) _scanTasks[recordId]!.error = null;
    } else {
      _scanTasks[recordId] = ScanTaskProgress(
        recordId: recordId,
        type: taskType,
        progress: progress,
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
    _settleScanWaiters();
    _scanTasks.clear();
    _analyzeTasks.clear();
    _deduplicateTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(
    String recordId, {
    bool isInitializeTask = false,
    bool force = false,
  }) async {
    if (isInitializeTask) {
      if (_navigate case final navigate?) {
        navigate('/scanning');
      } else {
        $$replace('/scanning');
      }
    }
    _scanCompleters.remove(recordId)?.complete(TaskStatus.cancelled);
    final scanId = '${DateTime.now().microsecondsSinceEpoch}-${++_nextScanId}';
    _scanIds[recordId] = scanId;
    _updateScanProgress(
      recordId,
      ScanTaskType.indexFiles,
      0,
      0,
      TaskStatus.working,
      isInitializeTask,
    );
    final request = ScanAudioLibraryRequest(
      force: force,
      scanId: scanId,
    );
    if (_sendScanRequest case final send?) {
      send(request);
    } else {
      request.sendSignalToRust();
    }
  }

  Future<void> analyzeLibrary(String recordId, [bool initialize = false]) async {
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
    ).sendSignalToRust();
  }

  Future<void> deduplicateLibrary(String recordId) async {
    _deduplicateCompleters.putIfAbsent(recordId, Completer<void>.new);
    double workloadFactor = 0.75;
    String similarityLevel =
        await $settingsManager.getValue<String>(kDeduplicateThresholdKey) ??
        "0.85";
    double similarityThreshold = double.tryParse(similarityLevel) ?? 0.85;

    DeduplicateAudioLibraryRequest(
      similarityThreshold: similarityThreshold,
      workloadFactor: workloadFactor,
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
    CancelTaskRequest(
      recordId: recordId,
      rType: type,
      scanId: _scanIds[recordId],
    ).sendSignalToRust();
  }

  void _settleScanWaiters() {
    for (final completer in _scanCompleters.values) {
      completer.complete(TaskStatus.cancelled);
    }
    _scanCompleters.clear();
    _scanIds.clear();
  }

  @override
  void dispose() {
    _settleScanWaiters();
    _libraryContentSubscription?.cancel();
    _scanProgressSubscription?.cancel();
    _scanResultSubscription?.cancel();
    _libraryTaskSubscription?.cancel();
    _requestFailureSubscription?.cancel();
    _failureFeedback.close();
    super.dispose();
  }
}
