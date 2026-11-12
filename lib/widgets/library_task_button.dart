import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../utils/l10n.dart';
import '../utils/platform.dart';
import '../utils/router/navigation.dart';
import '../screens/settings_library/widgets/progress_button.dart';
import '../bindings/bindings.dart';
import '../providers/library_manager.dart';
import '../providers/library_open_state.dart';
import '../utils/dialogs/information/error.dart';
import '../utils/api/request_failure.dart';
import '../utils/api/call_id.dart';

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

class LibraryTaskButton extends StatefulWidget {
  final String title;
  final String progressTitle;
  final Future<void> Function(LibraryManagerProvider, String, Uint64 callId)
  onPressedStart;
  final Future<void> Function(LibraryManagerProvider, String, Uint64 callId)?
  onContextMenu;
  final void Function(LibraryManagerProvider, String) onPressedCancel;
  final bool Function(bool, bool, bool) isTaskWorking;
  final double? progress;

  const LibraryTaskButton({
    required this.title,
    required this.progressTitle,
    required this.onPressedStart,
    required this.onPressedCancel,
    required this.isTaskWorking,
    required this.progress,
    this.onContextMenu,
    super.key,
  });

  @override
  State<LibraryTaskButton> createState() => _LibraryTaskButtonState();
}

class _LibraryTaskButtonState extends State<LibraryTaskButton> {
  StreamSubscription<RequestFailure>? _failureSubscription;
  final Set<int> _pendingCallIds = {};

  @override
  void initState() {
    super.initState();
    _failureSubscription = context
        .read<LibraryManagerProvider>()
        .failureFeedback
        .listen((failure) {
          if (!mounted ||
              failure.callId == null ||
              !_pendingCallIds.remove(failure.callId)) {
            return;
          }
          showErrorDialog(
            context: context,
            title: S.of(context).unavailable,
            errorMessage: requestFailureMessage(S.of(context), failure),
          );
        });
  }

  @override
  void dispose() {
    _failureSubscription?.cancel();
    super.dispose();
  }

  void _startTask(
    LibraryManagerProvider libraryManager,
    String recordId,
    Future<void> Function(LibraryManagerProvider, String, Uint64 callId) start,
  ) {
    final callId = nextCallId();
    _pendingCallIds.add(callId.toInt());
    unawaited(start(libraryManager, recordId, callId));
  }

  @override
  Widget build(BuildContext context) {
    final libraryPath = Provider.of<LibraryOpenStateProvider>(
      context,
      listen: true,
    );
    final recordId = libraryPath.openRecord?.recordId;

    if (!libraryPath.hasCatalog || recordId == null) {
      return Container();
    }

    final libraryManager = Provider.of<LibraryManagerProvider>(
      context,
      listen: true,
    );

    final scanProgress = libraryManager.getScanTaskProgress(recordId);
    final analyzeProgress = libraryManager.getAnalyzeTaskProgress(recordId);
    final deduplicateProgress = libraryManager.getDeduplicateTaskProgress(
      recordId,
    );

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyzeWorking = analyzeProgress?.status == TaskStatus.working;
    final deduplicateWorking =
        deduplicateProgress?.status == TaskStatus.working;

    final isWorking = scanWorking || analyzeWorking || deduplicateWorking;

    return isWorking &&
            widget.isTaskWorking(
              scanWorking,
              analyzeWorking,
              deduplicateWorking,
            )
        ? ProgressButton(
            title: widget.progressTitle,
            onPressed: () => widget.onPressedCancel(libraryManager, recordId),
            progress: widget.progress,
            filled: false,
          )
        : GestureDetector(
            onSecondaryTapUp: isWorking || widget.onContextMenu == null
                ? null
                : (details) {
                    _startTask(libraryManager, recordId, widget.onContextMenu!);
                  },
            onLongPressEnd: isWorking || isDesktop
                ? null
                : (details) {
                    _startTask(libraryManager, recordId, widget.onContextMenu!);
                  },
            child: Button(
              onPressed: isWorking
                  ? null
                  : () => _startTask(
                      libraryManager,
                      recordId,
                      widget.onPressedStart,
                    ),
              child: Text(widget.title),
            ),
          );
  }
}

class ScanLibraryButton extends StatelessWidget {
  final String? title;
  final void Function()? onFinished;

  const ScanLibraryButton({super.key, this.title, this.onFinished});

  @override
  Widget build(BuildContext context) {
    return LibraryTaskButton(
      title: title ?? S.of(context).scan,
      progressTitle: S.of(context).scanning,
      progress: null,
      onPressedCancel: (libraryManager, recordId) async {
        final confirm = await showCancelDialog(context);

        if (confirm == true) {
          libraryManager.cancelTask(recordId, CancelTaskType.scanAudioLibrary);
        }
      },
      onPressedStart: (libraryManager, recordId, callId) async {
        libraryManager.scanLibrary(
          recordId,
          isInitializeTask: false,
          callId: callId,
        );
        final status = await libraryManager.waitForScanToComplete(recordId);
        if (status == TaskStatus.failed && context.mounted) {
          await showErrorDialog(
            context: context,
            title: S.of(context).scanLibrary,
            errorMessage:
                libraryManager.getScanTaskProgress(recordId)?.error ??
                S.of(context).unknownError,
          );
        }
        if (status == TaskStatus.finished && onFinished != null) {
          onFinished!();
        }
      },
      onContextMenu: (libraryManager, recordId, callId) async {
        libraryManager.scanLibrary(
          recordId,
          isInitializeTask: false,
          force: true,
          callId: callId,
        );
        final status = await libraryManager.waitForScanToComplete(recordId);
        if (status == TaskStatus.failed && context.mounted) {
          await showErrorDialog(
            context: context,
            title: S.of(context).scanLibrary,
            errorMessage:
                libraryManager.getScanTaskProgress(recordId)?.error ??
                S.of(context).unknownError,
          );
        }
        if (status == TaskStatus.finished && onFinished != null) {
          onFinished!();
        }
      },
      isTaskWorking: (scanWorking, analyzeWorking, deduplicateWorking) =>
          scanWorking,
    );
  }
}

class AnalyzeLibraryButton extends StatelessWidget {
  final String? title;
  final void Function()? onFinished;

  const AnalyzeLibraryButton({super.key, this.title, this.onFinished});

  @override
  Widget build(BuildContext context) {
    final libraryManager = Provider.of<LibraryManagerProvider>(context);
    final recordId = Provider.of<LibraryOpenStateProvider>(
      context,
    ).openRecord?.recordId;
    final analyzeProgress = recordId == null
        ? null
        : libraryManager.getAnalyzeTaskProgress(recordId);

    final progress = analyzeProgress == null
        ? null
        : analyzeProgress.progress / analyzeProgress.total;

    return LibraryTaskButton(
      title: title ?? S.of(context).analyze,
      progressTitle: S.of(context).analyzing,
      onPressedCancel: (libraryManager, recordId) async {
        final confirm = await showCancelDialog(context);

        if (confirm == true) {
          libraryManager.cancelTask(
            recordId,
            CancelTaskType.analyzeAudioLibrary,
          );
        }
      },
      onPressedStart: (libraryManager, recordId, callId) async {
        libraryManager.analyzeLibrary(recordId, false, callId);
        await libraryManager.waitForAnalyzeToComplete(recordId);

        if (onFinished != null) {
          onFinished!();
        }
      },
      isTaskWorking: (scanWorking, analyzeWorking, deduplicateWorking) =>
          analyzeWorking,
      progress: progress,
    );
  }
}

class DeduplicateLibraryButton extends StatelessWidget {
  final String? title;
  final void Function()? onFinished;

  const DeduplicateLibraryButton({super.key, this.title, this.onFinished});

  @override
  Widget build(BuildContext context) {
    final libraryManager = Provider.of<LibraryManagerProvider>(context);
    final recordId = Provider.of<LibraryOpenStateProvider>(
      context,
    ).openRecord?.recordId;
    final deduplicateProgress = recordId == null
        ? null
        : libraryManager.getDeduplicateTaskProgress(recordId);

    final progress = deduplicateProgress == null
        ? null
        : deduplicateProgress.progress / deduplicateProgress.total;

    return LibraryTaskButton(
      title: title ?? S.of(context).deduplicate,
      progressTitle: S.of(context).deduplicating,
      onPressedCancel: (libraryManager, recordId) async {
        final confirm = await showCancelDialog(context);

        if (confirm == true) {
          libraryManager.cancelTask(
            recordId,
            CancelTaskType.deduplicateAudioLibrary,
          );
        }
      },
      onPressedStart: (libraryManager, recordId, callId) async {
        libraryManager.deduplicateLibrary(recordId, callId: callId);
        await libraryManager.waitForDeduplicateToComplete(recordId);

        if (onFinished != null) {
          onFinished!();
        }
      },
      isTaskWorking: (scanWorking, analyzeWorking, deduplicateWorking) =>
          deduplicateWorking,
      progress: progress,
    );
  }
}
