import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../widgets/library_task_button.dart';

import '../unavailable_dialog_on_band.dart';

import './not_analysed_text.dart';

Future<String?> showNoAnalysisDialog(
  BuildContext context, [
  bool collection = false,
]) async {
  return showDialog<String>(
    context: context,
    builder: (context) => UnavailableDialogOnBand(
      icon: Symbols.cognition,
      child: ContentDialog(
        title: const Column(
          children: [
            SizedBox(height: 8),
            Text("Not Ready"),
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
          const AnalyseLibraryButton(),
          Button(
            child: const Text('Cancel'),
            onPressed: () => Navigator.pop(context, 'Cancel'),
          ),
        ],
      ),
    ),
  );
}
