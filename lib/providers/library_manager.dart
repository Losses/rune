import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../messages/library_manage.pb.dart';

enum TaskStatus { working, finished }

class AnalyseTaskProgress {
  final String path;
  int progress;
  int total;
  TaskStatus status;

  AnalyseTaskProgress({
    required this.path,
    this.progress = 0,
    this.total = 0,
    this.status = TaskStatus.working,
  });
}

class ScanTaskProgress {
  final String path;
  int progress;
  TaskStatus status;

  ScanTaskProgress({
    required this.path,
    this.progress = 0,
    this.status = TaskStatus.working,
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
          scanProgress.path, scanProgress.progress, TaskStatus.working);
    });

    _scanResultSubscription =
        ScanAudioLibraryResponse.rustSignalStream.listen((event) {
      final scanResult = event.message;
      _updateScanProgress(
          scanResult.path, scanResult.progress, TaskStatus.finished);
    });

    _analyseProgressSubscription =
        AnalyseAudioLibraryProgress.rustSignalStream.listen((event) {
      final analyseProgress = event.message;
      _updateAnalyseProgress(analyseProgress.path, analyseProgress.progress,
          analyseProgress.total, TaskStatus.working);
    });

    _analyseResultSubscription =
        AnalyseAudioLibraryResponse.rustSignalStream.listen((event) {
      final analyseResult = event.message;
      _updateAnalyseProgress(analyseResult.path, analyseResult.total,
          analyseResult.total, TaskStatus.finished);
    });
  }

  void _updateScanProgress(String path, int progress, TaskStatus status) {
    if (_scanTasks.containsKey(path)) {
      _scanTasks[path]!.progress = progress;
    } else {
      _scanTasks[path] =
          ScanTaskProgress(path: path, progress: progress, status: status);
    }
    notifyListeners();
  }

  void _updateAnalyseProgress(
      String path, int progress, int total, TaskStatus status) {
    if (_analyseTasks.containsKey(path)) {
      _analyseTasks[path]!.progress = progress;
      _analyseTasks[path]!.total = total;
    } else {
      _analyseTasks[path] = AnalyseTaskProgress(
          path: path, progress: progress, total: total, status: status);
    }
    notifyListeners();
  }

  void clearAll(String path) {
    _scanTasks.clear();
    _analyseTasks.clear();
    notifyListeners();
  }

  Future<void> scanLibrary(BuildContext context, String path) async {
    _updateScanProgress(path, 0, TaskStatus.working);
    ScanAudioLibraryRequest(path: path).sendSignalToRust();
  }

  Future<void> analyseLibrary(BuildContext context, String path) async {
    _updateAnalyseProgress(path, 0, -1, TaskStatus.working);
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
