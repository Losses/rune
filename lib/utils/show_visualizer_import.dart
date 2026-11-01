import 'package:fluent_ui/fluent_ui.dart';

import '../providers/visualizer_scenes.dart';
import 'l10n.dart';

/// Tells the user how importing a scene file went.
void showVisualizerImport(BuildContext context, VisualizerImport result) {
  final s = S.of(context);
  final error = result.error;
  displayInfoBar(
    context,
    builder: (context, close) => InfoBar(
      title: Text(
        error == null
            ? s.visualizerSceneImported(result.name)
            : s.visualizerSceneImportFailed(result.name),
      ),
      content: error == null ? null : Text(error),
      severity: error == null ? InfoBarSeverity.success : InfoBarSeverity.error,
      onClose: close,
    ),
  );
}
