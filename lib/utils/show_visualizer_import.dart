import 'package:fluent_ui/fluent_ui.dart';

import '../providers/visualizer_scenes.dart';
import 'l10n.dart';

/// Tells the user how importing a scene file went.
void showVisualizerImport(BuildContext context, VisualizerImport result) {
  final s = S.of(context);
  final problem = switch (result) {
    VisualizerImported() => null,
    VisualizerImportRejected(:final reason) => reason,
    VisualizerImportUnanswered() => 'No answer from the visualizer',
  };
  displayInfoBar(
    context,
    builder: (context, close) => InfoBar(
      title: Text(
        problem == null
            ? s.visualizerSceneImported(result.name)
            : s.visualizerSceneImportFailed(result.name),
      ),
      content: problem == null ? null : Text(problem),
      severity: problem == null
          ? InfoBarSeverity.success
          : InfoBarSeverity.error,
      onClose: close,
    ),
  );
}
