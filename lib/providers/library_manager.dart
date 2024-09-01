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
}

class ScanTaskProgress {
  final String path;
  int progress;
  TaskStatus status;
  bool initialize;

  ScanTaskProgress({
    required this.path,
    this.progress = 0,
    this.status = TaskStatus.working,
    this.initialize = false,
  });
}

class LibraryManagerProvider with ChangeNotifier {
  final Map<String, AnalyseTaskProgress> _analyseTasks = {};
  final Map<String, ScanTaskProgress> _scanTasks = {};
  StreamSubscription? _scanProgressSubscription;
  StreamSubscription? _scanResultSubscription;
  StreamSubscription? _analyseProgressSubscription;
  StreamSubscription? _analyseResultSubscription;

  LibraryManagerProvider() {
    initListeners();
  }

  void initListeners() {
    _scanProgressSubscription =
        ScanAudioLibraryProgress.rustSignalStream.listen((event) {
      final scanProgress = event.message;
      _updateScanProgress(
          scanProgress.path,
          scanProgress.progress,
          TaskStatus.working,
          getScanTaskProgress(scanProgress.path)?.initialize ?? false);
    });

    _scanResultSubscription =
        ScanAudioLibraryResponse.rustSignalStream.listen((event) {
      final scanResult = event.message;
      final initialize =
          getScanTaskProgress(scanResult.path)?.initialize ?? false;
      _updateScanProgress(scanResult.path, scanResult.progress,
          TaskStatus.finished, initialize);

      if (initialize) {
        analyseLibrary(scanResult.path);
      }
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
    });
  }

  void _updateScanProgress(
      String path, int progress, TaskStatus status, bool initialize) {
    if (_scanTasks.containsKey(path)) {
      _scanTasks[path]!.progress = progress;
    } else {
      _scanTasks[path] = ScanTaskProgress(
          path: path,
          progress: progress,
          status: status,
          initialize: initialize);
    }
    notifyListeners();
  }

  void _updateAnalyseProgress(String path, int progress, int total,
      TaskStatus status, bool initialize) {
    if (_analyseTasks.containsKey(path)) {
      _analyseTasks[path]!.progress = progress;
      _analyseTasks[path]!.total = total;
    } else {
      _analyseTasks[path] = AnalyseTaskProgress(
          path: path,
          progress: progress,
          total: total,
          status: status,
          initialize: initialize);
    }
    notifyListeners();
  }

  void clearAll(String path) {
    _scanTasks.clear();
    _analyseTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(String path, [bool initialize = false]) async {
    _updateScanProgress(path, 0, TaskStatus.working, initialize);
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

  @override
  void dispose() {
    _scanProgressSubscription?.cancel();
    _scanResultSubscription?.cancel();
    _analyseProgressSubscription?.cancel();
    _analyseResultSubscription?.cancel();
    super.dispose();
  }
}
