import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../providers/library_path.dart';
import '../../../providers/library_manager.dart';

class NotAnalyzedText extends StatelessWidget {
  final bool? collection;

  const NotAnalyzedText({
    super.key,
    required this.collection,
  });

  @override
  Widget build(BuildContext context) {
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final itemPath = libraryPath.currentPath ?? '';

    final scanProgress = libraryManager.getScanTaskProgress(itemPath);
    final analyzeProgress = libraryManager.getAnalyzeTaskProgress(itemPath);

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyzeWorking = analyzeProgress?.status == TaskStatus.working;

    String getMessage(bool isCollection) {
      final baseMessage = isCollection
          ? S.of(context).noRoamingCollection
          : S.of(context).noRoamingTrack;

      if (scanWorking) {
        return S.of(context).noAnalysisScanning(baseMessage);
      }

      if (analyzeWorking) {
        return S.of(context).noAnalysisAnalyzing(baseMessage);
      }

      return S.of(context).noAnalysisDefault(baseMessage);
    }

    return Text(getMessage(collection ?? false));
  }
}
