import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_path.dart';
import '../../../providers/library_manager.dart';

class NotAnalysedText extends StatelessWidget {
  final bool? collection;

  const NotAnalysedText({
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
    final analyseProgress = libraryManager.getAnalyseTaskProgress(itemPath);

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyseWorking = analyseProgress?.status == TaskStatus.working;

    String getMessage(bool isCollection) {
      final baseMessage = isCollection
          ? "Unable to start roaming. Tracks in the collection hasn't been analyzed yet."
          : "Unable to start roaming. This track hasn't been analyzed yet.";

      if (scanWorking) {
        return "$baseMessage The library is being scanned, so analysis cannot be performed.";
      }

      if (analyseWorking) {
        return "$baseMessage The library is being analyzed; please wait until the process finished.";
      }

      return "$baseMessage Please analyze your library for the best experience.";
    }

    return Text(getMessage(collection ?? false));
  }
}
