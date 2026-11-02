import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../bindings/bindings.dart';
import '../constants/configurations.dart';
import '../constants/settings_manager.dart';

enum TaskStatus { working, finished, cancelled, failed }

class AnalyzeTaskProgress {
  final String path;
  int progress;
  int total;
  TaskStatus status;
  bool isInitializeTask;

  AnalyzeTaskProgress({
    required this.path,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
    this.isInitializeTask = false,
  });

  @override
  String toString() {
    return 'AnalyzeTaskProgress(path: $path, progress: $progress, total: $total, status: $status, initialize: $isInitializeTask)';
  }
}

class ScanTaskProgress {
  final String path;
  ScanTaskType type;
  int progress;
  TaskStatus status;
  bool initialize;
  String? error;

  ScanTaskProgress({
    required this.path,
    required this.type,
    this.progress = 0,
    this.status = TaskStatus.working,
    this.initialize = false,
  });

  @override
  String toString() {
    return 'ScanTaskProgress(path: $path, progress: $progress, status: $status, type: $type, initialize: $initialize)';
  }
}

class DeduplicateTaskProgress {
  final String path;
  int progress;
  int total;
  TaskStatus status;

  DeduplicateTaskProgress({
    required this.path,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
  });

  @override
  String toString() {
    return 'DeduplicateTaskProgress(path: $path, progress: $progress, total: $total, status: $status)';
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
          if (_scanIds[scanProgress.path] != scanProgress.scanId ||
              getScanTaskProgress(scanProgress.path)?.status !=
                  TaskStatus.working) {
            return;
          }
          _updateScanProgress(
            scanProgress.path,
            scanProgress.task,
            scanProgress.progress,
            scanProgress.total,
            TaskStatus.working,
            getScanTaskProgress(scanProgress.path)?.initialize ?? false,
          );
          CollectionCache().clearAll();
        });

    _scanResultSubscription = ScanAudioLibraryResponse.rustSignalStream.listen((
      event,
    ) {
      final scanResult = event.message;
      if (_scanIds[scanResult.path] != scanResult.scanId) return;
      final status = switch (scanResult.status) {
        "success" => TaskStatus.finished,
        "cancelled" => TaskStatus.cancelled,
        _ => TaskStatus.failed,
      };
      final initialize =
          getScanTaskProgress(scanResult.path)?.initialize ?? false;
      _updateScanProgress(
        scanResult.path,
        ScanTaskType.scanCoverArts,
        scanResult.progress,
        0,
        status,
        initialize,
      );

      getScanTaskProgress(scanResult.path)?.error = scanResult.error;
      notifyListeners();
      if (initialize && status == TaskStatus.finished) {
        if (_navigate case final navigate?) {
          navigate("/library");
        } else {
          $$replace("/library");
        }
      }

      // Complete the scan task
      _scanCompleters[scanResult.path]?.complete(status);
      _scanCompleters.remove(scanResult.path);
      CollectionCache().clearAll();
    });

    _analyzeProgressSubscription = AnalyzeAudioLibraryProgress.rustSignalStream
        .listen((event) {
          final analyzeProgress = event.message;
          _updateAnalyzeProgress(
            analyzeProgress.path,
            analyzeProgress.progress,
            analyzeProgress.total,
            TaskStatus.working,
            getAnalyzeTaskProgress(analyzeProgress.path)?.isInitializeTask ??
                false,
          );
        });

    _analyzeResultSubscription = AnalyzeAudioLibraryResponse.rustSignalStream
        .listen((event) {
          final analyzeResult = event.message;
          _updateAnalyzeProgress(
            analyzeResult.path,
            analyzeResult.total,
            analyzeResult.total,
            TaskStatus.finished,
            getAnalyzeTaskProgress(analyzeResult.path)?.isInitializeTask ??
                false,
          );

          // Complete the analyze task
          _analyzeCompleters[analyzeResult.path]?.complete();
          _analyzeCompleters.remove(analyzeResult.path);
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
            cancelResponse.path,
            0,
            0,
            TaskStatus.cancelled,
            false,
          );
        } else if (cancelResponse.rType ==
            CancelTaskType.deduplicateAudioLibrary) {
          _updateDeduplicateProgress(
            cancelResponse.path,
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
            deduplicateProgress.path,
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
            deduplicateResult.path,
            100,
            100,
            TaskStatus.finished,
          );

          // Complete deduplicate task
          _deduplicateCompleters[deduplicateResult.path]?.complete();
          _deduplicateCompleters.remove(deduplicateResult.path);
        });
  }

  void _updateScanProgress(
    String path,
    ScanTaskType taskType,
    int progress,
    int total,
    TaskStatus status,
    bool initialize,
  ) {
    if (_scanTasks.containsKey(path)) {
      _scanTasks[path]!.progress = progress;
      _scanTasks[path]!.status = status;
      _scanTasks[path]!.type = taskType;
      _scanTasks[path]!.initialize = initialize;
      if (status == TaskStatus.working) _scanTasks[path]!.error = null;
    } else {
      _scanTasks[path] = ScanTaskProgress(
        path: path,
        type: taskType,
        progress: progress,
        status: status,
        initialize: initialize,
      );
    }
    notifyListeners();
  }

  void _updateAnalyzeProgress(
    String path,
    int progress,
    int total,
    TaskStatus status,
    bool initialize,
  ) {
    if (_analyzeTasks.containsKey(path)) {
      _analyzeTasks[path]!.progress = progress;
      _analyzeTasks[path]!.total = total;
      _analyzeTasks[path]!.status = status;
    } else {
      _analyzeTasks[path] = AnalyzeTaskProgress(
        path: path,
        progress: progress,
        total: total,
        status: status,
        isInitializeTask: initialize,
      );
    }
    notifyListeners();
  }

  void _updateDeduplicateProgress(
    String path,
    int progress,
    int total,
    TaskStatus status,
  ) {
    if (_deduplicateTasks.containsKey(path)) {
      _deduplicateTasks[path]!.progress = progress;
      _deduplicateTasks[path]!.total = total;
      _deduplicateTasks[path]!.status = status;
    } else {
      _deduplicateTasks[path] = DeduplicateTaskProgress(
        path: path,
        progress: progress,
        total: total,
        status: status,
      );
    }
    notifyListeners();
  }

  void clearAll(String path) {
    _settleScanWaiters();
    _scanTasks.clear();
    _analyzeTasks.clear();
    _deduplicateTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(
    String path, {
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
    _scanCompleters.remove(path)?.complete(TaskStatus.cancelled);
    final scanId = '${DateTime.now().microsecondsSinceEpoch}-${++_nextScanId}';
    _scanIds[path] = scanId;
    _updateScanProgress(
      path,
      ScanTaskType.indexFiles,
      0,
      0,
      TaskStatus.working,
      isInitializeTask,
    );
    final request = ScanAudioLibraryRequest(
      path: path,
      force: force,
      scanId: scanId,
    );
    if (_sendScanRequest case final send?) {
      send(request);
    } else {
      request.sendSignalToRust();
    }
  }

  Future<void> analyzeLibrary(String path, [bool initialize = false]) async {
    _updateAnalyzeProgress(path, 0, -1, TaskStatus.working, initialize);
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
      path: path,
      computingDevice: computingDevice == 'gpu'
          ? ComputingDeviceRequest.gpu
          : ComputingDeviceRequest.cpu,
      workloadFactor: workloadFactor,
    ).sendSignalToRust();
  }

  Future<void> deduplicateLibrary(String path) async {
    _updateDeduplicateProgress(path, 0, -1, TaskStatus.working);

    double workloadFactor = 0.75;
    String similarityLevel =
        await $settingsManager.getValue<String>(kDeduplicateThresholdKey) ??
        "0.85";
    double similarityThreshold = double.tryParse(similarityLevel) ?? 0.85;

    DeduplicateAudioLibraryRequest(
      path: path,
      similarityThreshold: similarityThreshold,
      workloadFactor: workloadFactor,
    ).sendSignalToRust();
  }

  ScanTaskProgress? getScanTaskProgress(String? path) {
    return _scanTasks[path];
  }

  AnalyzeTaskProgress? getAnalyzeTaskProgress(String path) {
    return _analyzeTasks[path];
  }

  DeduplicateTaskProgress? getDeduplicateTaskProgress(String path) {
    return _deduplicateTasks[path];
  }

  Future<TaskStatus> waitForScanToComplete(String path) {
    final taskProgress = getScanTaskProgress(path);
    if (taskProgress == null || taskProgress.status != TaskStatus.working) {
      return Future.value(taskProgress?.status ?? TaskStatus.finished);
    }

    final existed = _scanCompleters[path];
    if (existed != null) return existed.future;

    _scanCompleters[path] = Completer<TaskStatus>();
    return _scanCompleters[path]!.future;
  }

  Future<void> waitForAnalyzeToComplete(String path) {
    final taskProgress = getAnalyzeTaskProgress(path);
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
      return Future.value();
    }

    final existed = _analyzeCompleters[path];
    if (existed != null) return existed.future;

    _analyzeCompleters[path] = Completer<void>();
    return _analyzeCompleters[path]!.future;
  }

  Future<void> waitForDeduplicateToComplete(String path) {
    final taskProgress = getDeduplicateTaskProgress(path);
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
      return Future.value();
    }

    final existed = _deduplicateCompleters[path];
    if (existed != null) return existed.future;

    _deduplicateCompleters[path] = Completer<void>();
    return _deduplicateCompleters[path]!.future;
  }

  Future<void> cancelTask(String path, CancelTaskType type) async {
    CancelTaskRequest(
      path: path,
      rType: type,
      scanId: _scanIds[path],
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
