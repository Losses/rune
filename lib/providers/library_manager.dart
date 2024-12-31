import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../utils/settings_manager.dart';
import '../utils/router/navigation.dart';
import '../screens/collection/utils/collection_data_provider.dart';
import '../messages/library_manage.pb.dart';
import '../constants/configurations.dart';

enum TaskStatus { working, finished, cancelled }

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

class LibraryManagerProvider with ChangeNotifier {
  final Map<String, AnalyzeTaskProgress> _analyzeTasks = {};
  final Map<String, ScanTaskProgress> _scanTasks = {};
  StreamSubscription? _scanProgressSubscription;
  StreamSubscription? _scanResultSubscription;
  StreamSubscription? _analyzeProgressSubscription;
  StreamSubscription? _analyzeResultSubscription;
  StreamSubscription? _cancelTaskSubscription;

  final Map<String, Completer<void>> _scanCompleters = {};
  final Map<String, Completer<void>> _analyzeCompleters = {};

  LibraryManagerProvider() {
    initListeners();
  }

  void initListeners() {
    _scanProgressSubscription =
        ScanAudioLibraryProgress.rustSignalStream.listen((event) {
      final scanProgress = event.message;
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

    _scanResultSubscription =
        ScanAudioLibraryResponse.rustSignalStream.listen((event) {
      final scanResult = event.message;
      final initialize =
          getScanTaskProgress(scanResult.path)?.initialize ?? false;
      _updateScanProgress(
        scanResult.path,
        ScanTaskType.ScanCoverArts,
        scanResult.progress,
        0,
        TaskStatus.finished,
        initialize,
      );

      if (initialize) {
        $$replace("/library");
      }

      // Complete the scan task
      _scanCompleters[scanResult.path]?.complete();
      _scanCompleters.remove(scanResult.path);
      CollectionCache().clearAll();
    });

    _analyzeProgressSubscription =
        AnalyzeAudioLibraryProgress.rustSignalStream.listen((event) {
      final analyzeProgress = event.message;
      _updateAnalyzeProgress(
        analyzeProgress.path,
        analyzeProgress.progress,
        analyzeProgress.total,
        TaskStatus.working,
        getAnalyzeTaskProgress(analyzeProgress.path)?.isInitializeTask ?? false,
      );
    });

    _analyzeResultSubscription =
        AnalyzeAudioLibraryResponse.rustSignalStream.listen((event) {
      final analyzeResult = event.message;
      _updateAnalyzeProgress(
          analyzeResult.path,
          analyzeResult.total,
          analyzeResult.total,
          TaskStatus.finished,
          getAnalyzeTaskProgress(analyzeResult.path)?.isInitializeTask ??
              false);

      // Complete the analyze task
      _analyzeCompleters[analyzeResult.path]?.complete();
      _analyzeCompleters.remove(analyzeResult.path);
    });

    _cancelTaskSubscription =
        CancelTaskResponse.rustSignalStream.listen((event) {
      final cancelResponse = event.message;
      if (cancelResponse.success) {
        if (cancelResponse.type == CancelTaskType.ScanAudioLibrary) {
          _updateScanProgress(
            cancelResponse.path,
            ScanTaskType.IndexFiles,
            0,
            0,
            TaskStatus.cancelled,
            false,
          );
        } else if (cancelResponse.type == CancelTaskType.AnalyzeAudioLibrary) {
          _updateAnalyzeProgress(
            cancelResponse.path,
            0,
            0,
            TaskStatus.cancelled,
            false,
          );
        }
      }
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

  void _updateAnalyzeProgress(String path, int progress, int total,
      TaskStatus status, bool initialize) {
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

  void clearAll(String path) {
    _scanTasks.clear();
    _analyzeTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(
    String path, {
    bool isInitializeTask = false,
    bool force = false,
  }) async {
    if (isInitializeTask) {
      $$replace('/scanning');
    }
    _updateScanProgress(
      path,
      ScanTaskType.IndexFiles,
      0,
      0,
      TaskStatus.working,
      isInitializeTask,
    );
    ScanAudioLibraryRequest(
      path: path,
      force: force,
    ).sendSignalToRust();
  }

  Future<void> analyzeLibrary(String path, [bool initialize = false]) async {
    _updateAnalyzeProgress(path, 0, -1, TaskStatus.working, initialize);
    final computingDevice =
        await SettingsManager().getValue<String>(kAnalysisComputingDeviceKey);

    double workloadFactor = 0.75;

    String? performanceLevel =
        await SettingsManager().getValue<String>(kAnalysisPerformanceLevelKey);

    if (performanceLevel == "balance") {
      workloadFactor = 0.5;
    }

    if (performanceLevel == "battery") {
      workloadFactor = 0.25;
    }

    AnalyzeAudioLibraryRequest(
      path: path,
      computingDevice:
          computingDevice == 'gpu' ? ComputingDevice.Gpu : ComputingDevice.Cpu,
      workloadFactor: workloadFactor,
    ).sendSignalToRust();
  }

  ScanTaskProgress? getScanTaskProgress(String? path) {
    return _scanTasks[path];
  }

  AnalyzeTaskProgress? getAnalyzeTaskProgress(String path) {
    return _analyzeTasks[path];
  }

  Future<void> waitForScanToComplete(String path) {
    final taskProgress = getScanTaskProgress(path);
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
      return Future.value();
    }

    final existed = _scanCompleters[path];
    if (existed != null) return existed.future;

    _scanCompleters[path] = Completer<void>();
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

  Future<void> cancelTask(String path, CancelTaskType type) async {
    CancelTaskRequest(path: path, type: type).sendSignalToRust();
  }

  @override
  void dispose() {
    _scanProgressSubscription?.cancel();
    _scanResultSubscription?.cancel();
    _analyzeProgressSubscription?.cancel();
    _analyzeResultSubscription?.cancel();
    _cancelTaskSubscription?.cancel();
    super.dispose();
  }
}
