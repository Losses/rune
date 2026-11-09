import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

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
  StreamSubscription? _analyzeProgressSubscription;
  StreamSubscription? _analyzeResultSubscription;
  StreamSubscription? _cancelTaskSubscription;
  StreamSubscription? _deduplicateProgressSubscription;
  StreamSubscription? _deduplicateResultSubscription;

  final Map<String, Completer<TaskStatus>> _scanCompleters = {};
  final Map<String, String> _scanIds = {};
  int _nextScanId = 0;
  final Map<String, Completer<void>> _analyzeCompleters = {};
  final Map<String, Completer<void>> _deduplicateCompleters = {};

  final void Function(ScanAudioLibraryRequest)? _sendScanRequest;
  final void Function(String)? _navigate;

  LibraryManagerProvider({
    void Function(ScanAudioLibraryRequest)? sendScanRequest,
    void Function(String)? navigate,
  }) : _sendScanRequest = sendScanRequest,
       _navigate = navigate {
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

    _analyzeProgressSubscription = AnalyzeAudioLibraryProgress.rustSignalStream
        .listen((event) {
          final analyzeProgress = event.message;
          _updateAnalyzeProgress(
            analyzeProgress.recordId,
            analyzeProgress.progress,
            analyzeProgress.total,
            TaskStatus.working,
            getAnalyzeTaskProgress(analyzeProgress.recordId)?.isInitializeTask ??
                false,
          );
        });

    _analyzeResultSubscription = AnalyzeAudioLibraryResponse.rustSignalStream
        .listen((event) {
          final analyzeResult = event.message;
          _updateAnalyzeProgress(
            analyzeResult.recordId,
            analyzeResult.total,
            analyzeResult.total,
            TaskStatus.finished,
            getAnalyzeTaskProgress(analyzeResult.recordId)?.isInitializeTask ??
                false,
          );

          // Complete the analyze task
          _analyzeCompleters[analyzeResult.recordId]?.complete();
          _analyzeCompleters.remove(analyzeResult.recordId);
        });

    _cancelTaskSubscription = CancelTaskResponse.rustSignalStream.listen((
      event,
    ) {
      final cancelResponse = event.message;
      if (cancelResponse.success) {
        if (cancelResponse.rType == CancelTaskType.scanAudioLibrary) {
          // The supervised worker publishes the terminal state with its scan ID.
        } else if (cancelResponse.rType == CancelTaskType.analyzeAudioLibrary) {
          _updateAnalyzeProgress(
            cancelResponse.recordId,
            0,
            0,
            TaskStatus.cancelled,
            false,
          );
        } else if (cancelResponse.rType ==
            CancelTaskType.deduplicateAudioLibrary) {
          _updateDeduplicateProgress(
            cancelResponse.recordId,
            0,
            0,
            TaskStatus.cancelled,
          );
        }
      }
    });

    _deduplicateProgressSubscription = DeduplicateAudioLibraryProgress
        .rustSignalStream
        .listen((event) {
          final deduplicateProgress = event.message;
          _updateDeduplicateProgress(
            deduplicateProgress.recordId,
            deduplicateProgress.progress,
            deduplicateProgress.total,
            TaskStatus.working,
          );
        });

    _deduplicateResultSubscription = DeduplicateAudioLibraryResponse
        .rustSignalStream
        .listen((event) {
          final deduplicateResult = event.message;
          _updateDeduplicateProgress(
            deduplicateResult.recordId,
            100,
            100,
            TaskStatus.finished,
          );

          // Complete deduplicate task
          _deduplicateCompleters[deduplicateResult.recordId]?.complete();
          _deduplicateCompleters.remove(deduplicateResult.recordId);
        });
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
    _updateAnalyzeProgress(recordId, 0, -1, TaskStatus.working, initialize);
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
    _updateDeduplicateProgress(recordId, 0, -1, TaskStatus.working);

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
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
      return Future.value();
    }

    final existed = _analyzeCompleters[recordId];
    if (existed != null) return existed.future;

    _analyzeCompleters[recordId] = Completer<void>();
    return _analyzeCompleters[recordId]!.future;
  }

  Future<void> waitForDeduplicateToComplete(String recordId) {
    final taskProgress = getDeduplicateTaskProgress(recordId);
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
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
    _analyzeProgressSubscription?.cancel();
    _analyzeResultSubscription?.cancel();
    _cancelTaskSubscription?.cancel();
    _deduplicateProgressSubscription?.cancel();
    _deduplicateResultSubscription?.cancel();
    super.dispose();
  }
}
