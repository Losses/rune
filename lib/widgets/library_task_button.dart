import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../screens/settings_library/widgets/progress_button.dart';
import '../providers/library_manager.dart';
import '../providers/library_path.dart';

class LibraryTaskButton extends StatelessWidget {
  final String title;
  final String progressTitle;
  final Future<void> Function(LibraryManagerProvider, String) onPressedAction;
  final bool Function(bool, bool) isTaskWorking;
  final double? progress;

  const LibraryTaskButton({
    required this.title,
    required this.progressTitle,
    required this.onPressedAction,
    required this.isTaskWorking,
    required this.progress,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final itemPath = libraryPath.currentPath;

    if (itemPath == null) {
      return Container();
    }

    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);

    final scanProgress = libraryManager.getScanTaskProgress(itemPath);
    final analyseProgress = libraryManager.getAnalyseTaskProgress(itemPath);

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyseWorking = analyseProgress?.status == TaskStatus.working;

    final isWorking = scanWorking || analyseWorking;

    return isWorking && isTaskWorking(scanWorking, analyseWorking)
        ? ProgressButton(
            title: progressTitle,
            onPressed: null,
            progress: progress,
          )
        : Button(
            onPressed: isWorking
                ? null
                : () => onPressedAction(libraryManager, itemPath),
            child: Text(title),
          );
  }
}

class ScanLibraryButton extends StatelessWidget {
  final String? title;
  final void Function()? onFinished;

  const ScanLibraryButton({
    super.key,
    this.title,
    this.onFinished,
  });

  @override
  Widget build(BuildContext context) {
    return LibraryTaskButton(
      title: title ?? "Scan",
      progressTitle: "Scanning",
      progress: null,
      onPressedAction: (libraryManager, itemPath) async {
        libraryManager.scanLibrary(itemPath, false);
        await libraryManager.waitForScanToComplete(itemPath);

        if (onFinished != null) {
          onFinished!();
        }
      },
      isTaskWorking: (scanWorking, analyseWorking) => scanWorking,
    );
  }
}

class AnalyseLibraryButton extends StatelessWidget {
  final String? title;
  final void Function()? onFinished;

  const AnalyseLibraryButton({
    super.key,
    this.title,
    this.onFinished,
  });

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final itemPath = libraryPath.currentPath;

    if (itemPath == null) return Container();

    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);

    final analyseProgress = libraryManager.getAnalyseTaskProgress(itemPath);

    final progress = analyseProgress == null
        ? null
        : analyseProgress.progress / analyseProgress.total;

    return LibraryTaskButton(
      title: title ?? "Analyse",
      progressTitle: "Analysing",
      onPressedAction: (libraryManager, itemPath) async {
        libraryManager.analyseLibrary(itemPath, false);
        await libraryManager.waitForAnalyseToComplete(itemPath);

        if (onFinished != null) {
          onFinished!();
        }
      },
      isTaskWorking: (scanWorking, analyseWorking) => analyseWorking,
      progress: progress,
    );
  }
}
