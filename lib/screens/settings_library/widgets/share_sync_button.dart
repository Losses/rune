import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_manager.dart';
import '../../../utils/l10n.dart';

import 'progress_button.dart';

/// Starts a sync of a library with its shared folder, and shows a running
/// one the way the scan button shows a running scan.
class ShareSyncButton extends StatelessWidget {
  const ShareSyncButton({
    super.key,
    required this.libraryId,
    required this.onPressed,
  });

  final String libraryId;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    final syncing = context.select<LibraryManagerProvider, bool>(
      (libraryManager) => libraryManager.isShareSyncing(libraryId),
    );

    // A sync cannot be cancelled once the hub has started it, so the running
    // state offers nothing to press.
    return syncing
        ? ProgressButton(
            title: S.of(context).syncing,
            onPressed: null,
            progress: null,
            filled: false,
          )
        : Button(onPressed: onPressed, child: Text(S.of(context).syncNow));
  }
}
