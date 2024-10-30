import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../messages/library_manage.pb.dart';

enum TaskStatus { working, finished }

class AnalyseTaskProgress {
  final String path;
  int progress;
  int total;
  TaskStatus status;
  bool initialize;

  AnalyseTaskProgress({
    required this.path,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
    this.initialize = false,
  });

  @override
  String toString() {
    return 'AnalyseTaskProgress(path: $path, progress: $progress, total: $total, status: $status, initialize: $initialize)';
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
    return 'ScanTaskProgress(path: $path, progress: $progress, status: $status, initialize: $initialize)';
  }
}

class LibraryManagerProvider with ChangeNotifier {
  final Map<String, AnalyseTaskProgress> _analyseTasks = {};
  final Map<String, ScanTaskProgress> _scanTasks = {};
  StreamSubscription? _scanProgressSubscription;
  StreamSubscription? _scanResultSubscription;
  StreamSubscription? _analyseProgressSubscription;
  StreamSubscription? _analyseResultSubscription;

  final Map<String, Completer<void>> _scanCompleters = {};
  final Map<String, Completer<void>> _analyseCompleters = {};

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
          getScanTaskProgress(scanProgress.path)?.initialize ?? false);
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
        analyseLibrary(scanResult.path);
      }

      // Complete the scan task
      _scanCompleters[scanResult.path]?.complete();
      _scanCompleters.remove(scanResult.path);
    });

    _analyseProgressSubscription =
        AnalyseAudioLibraryProgress.rustSignalStream.listen((event) {
      final analyseProgress = event.message;
      _updateAnalyseProgress(
          analyseProgress.path,
          analyseProgress.progress,
          analyseProgress.total,
          TaskStatus.working,
          getAnalyseTaskProgress(analyseProgress.path)?.initialize ?? false);
    });

    _analyseResultSubscription =
        AnalyseAudioLibraryResponse.rustSignalStream.listen((event) {
      final analyseResult = event.message;
      _updateAnalyseProgress(
          analyseResult.path,
          analyseResult.total,
          analyseResult.total,
          TaskStatus.finished,
          getAnalyseTaskProgress(analyseResult.path)?.initialize ?? false);

      // Complete the analyse task
      _analyseCompleters[analyseResult.path]?.complete();
      _analyseCompleters.remove(analyseResult.path);
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

  void _updateAnalyseProgress(String path, int progress, int total,
      TaskStatus status, bool initialize) {
    if (_analyseTasks.containsKey(path)) {
      _analyseTasks[path]!.progress = progress;
      _analyseTasks[path]!.total = total;
      _analyseTasks[path]!.status = status;
    } else {
      _analyseTasks[path] = AnalyseTaskProgress(
        path: path,
        progress: progress,
        total: total,
        status: status,
        initialize: initialize,
      );
    }
    notifyListeners();
  }

  void clearAll(String path) {
    _scanTasks.clear();
    _analyseTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(String path, [bool initialize = false]) async {
    _updateScanProgress(
      path,
      ScanTaskType.IndexFiles,
      0,
      0,
      TaskStatus.working,
      initialize,
    );
    ScanAudioLibraryRequest(path: path).sendSignalToRust();
  }

  Future<void> analyseLibrary(String path, [bool initialize = false]) async {
    _updateAnalyseProgress(path, 0, -1, TaskStatus.working, initialize);
    AnalyseAudioLibraryRequest(path: path).sendSignalToRust();
  }

  ScanTaskProgress? getScanTaskProgress(String path) {
    return _scanTasks[path];
  }

  AnalyseTaskProgress? getAnalyseTaskProgress(String path) {
    return _analyseTasks[path];
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

  Future<void> waitForAnalyseToComplete(String path) {
    final taskProgress = getAnalyseTaskProgress(path);
    if (taskProgress == null || taskProgress.status == TaskStatus.finished) {
      return Future.value();
    }

    final existed = _analyseCompleters[path];
    if (existed != null) return existed.future;

    _analyseCompleters[path] = Completer<void>();
    return _analyseCompleters[path]!.future;
  }

  @override
  void dispose() {
    _scanProgressSubscription?.cancel();
    _scanResultSubscription?.cancel();
    _analyseProgressSubscription?.cancel();
    _analyseResultSubscription?.cancel();
    super.dispose();
  }
}
