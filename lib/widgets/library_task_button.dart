import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/router/navigation.dart';
import '../screens/settings_library/widgets/progress_button.dart';
import '../messages/library_manage.pbenum.dart';
import '../providers/library_manager.dart';
import '../providers/library_path.dart';
import '../utils/l10n.dart';

Future<bool?> showCancelDialog(BuildContext context) {
  return $showModal<bool>(
    context,
    (context, $close) => ContentDialog(
      title: Text(S.of(context).cancelTaskTitle),
      content: Text(S.of(context).cancelTaskSubtitle),
      actions: [
        FilledButton(
          child: Text(S.of(context).cancelTask),
          onPressed: () {
            $close(true);
          },
        ),
        Button(
          child: Text(S.of(context).continueTask),
          onPressed: () => $close(false),
        ),
      ],
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}

class LibraryTaskButton extends StatelessWidget {
  final String title;
  final String progressTitle;
  final Future<void> Function(LibraryManagerProvider, String) onPressedStart;
  final void Function(LibraryManagerProvider, String) onPressedCancel;
  final bool Function(bool, bool) isTaskWorking;
  final double? progress;

  const LibraryTaskButton({
    required this.title,
    required this.progressTitle,
    required this.onPressedStart,
    required this.onPressedCancel,
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
            onPressed: () => onPressedCancel(libraryManager, itemPath),
            progress: progress,
            filled: false,
          )
        : Button(
            onPressed: isWorking
                ? null
                : () => onPressedStart(libraryManager, itemPath),
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
      title: title ?? S.of(context).scan,
      progressTitle: S.of(context).scanning,
      progress: null,
      onPressedCancel: (libraryManager, itemPath) async {
        final confirm = await showCancelDialog(context);

        if (confirm == true) {
          libraryManager.cancelTask(itemPath, CancelTaskType.ScanAudioLibrary);
        }
      },
      onPressedStart: (libraryManager, itemPath) async {
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
      title: title ?? S.of(context).analyse,
      progressTitle: S.of(context).analysing,
      onPressedCancel: (libraryManager, itemPath) async {
        final confirm = await showCancelDialog(context);

        if (confirm == true) {
          libraryManager.cancelTask(
            itemPath,
            CancelTaskType.AnalyseAudioLibrary,
          );
        }
      },
      onPressedStart: (libraryManager, itemPath) async {
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
