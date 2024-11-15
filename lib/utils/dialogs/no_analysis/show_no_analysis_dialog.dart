import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/router/navigation.dart';

import '../../../widgets/library_task_button.dart';
import '../../../widgets/responsive_dialog_actions.dart';

import '../../../generated/l10n.dart';

import '../unavailable_dialog_on_band.dart';

import './not_analysed_text.dart';

Future<String?> showNoAnalysisDialog(
  BuildContext context, [
  bool collection = false,
]) async {
  return $showModal<String>(
    context,
    (context, $close) => UnavailableDialogOnBand(
      $close: $close,
      icon: Symbols.cognition,
      child: ContentDialog(
        title: Column(
          children: [
            const SizedBox(height: 8),
            Text(S.of(context).notReady),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            NotAnalysedText(
              collection: collection,
            ),
            const SizedBox(height: 4),
          ],
        ),
        actions: [
          ResponsiveDialogActions(
            const AnalyseLibraryButton(),
            Button(
              child: Text(S.of(context).cancel),
              onPressed: () => $close('Cancel'),
            ),
          ),
        ],
      ),
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
