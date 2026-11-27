import 'package:fluent_ui/fluent_ui.dart';

import '../providers/visualizer_scenes.dart';
import 'dialogs/information/error.dart';
import 'dialogs/information/information.dart';
import 'l10n.dart';

/// Tells the user how importing a scene file went.
Future<void> showVisualizerImport(
  BuildContext context,
  VisualizerImport result,
) async {
  final s = S.of(context);
  final problem = switch (result) {
    VisualizerImported() => null,
    VisualizerImportRejected(:final reason) => reason,
    VisualizerImportUnanswered() => s.visualizerNoAnswer,
  };
  if (problem == null) {
    await showInformationDialog(
      context: context,
      title: s.visualizerScenes,
      subtitle: s.visualizerSceneImported(result.name),
    );
  } else {
    await showErrorDialog(
      context: context,
      title: s.visualizerSceneImportFailed(result.name),
      errorMessage: problem,
    );
  }
}
