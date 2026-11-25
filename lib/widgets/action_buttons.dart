import 'package:fluent_ui/fluent_ui.dart';

import '../widgets/library_task_button.dart';
import '../utils/l10n.dart';

class ActionButtons extends StatelessWidget {
  const ActionButtons({
    super.key,
    required this.reloadData,
    required this.hasRecommendation,
  });

  final VoidCallback reloadData;
  final bool hasRecommendation;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        ScanLibraryButton(
          title: S.of(context).scanLibrary,
          onFinished: reloadData,
        ),
        if (hasRecommendation) ...[
          const SizedBox(width: 12),
          AnalyzeLibraryButton(
            title: S.of(context).analyzeTracks,
            onFinished: reloadData,
          ),
        ]
      ],
    );
  }
}
