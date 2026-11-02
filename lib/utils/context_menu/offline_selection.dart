import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/library_path.dart';
import '../api/set_download_rule.dart';
import '../l10n.dart';

bool supportsOfflineSelection(BuildContext context) {
  final path = context.read<LibraryPathProvider>().currentPath;
  return path != null && !path.startsWith('@RR|') && !path.startsWith('@LR|');
}

MenuFlyoutSubItem offlineSelectionMenu(
  BuildContext context,
  CollectionType type,
  int id,
) {
  final s = S.of(context);
  return MenuFlyoutSubItem(
    leading: const Icon(Symbols.download),
    text: Text(s.offlineAvailability),
    items: (_) => [
      MenuFlyoutItem(
        text: Text(s.keepAvailableOffline),
        onPressed: () => _saveSelection(context, type, id, true),
      ),
      MenuFlyoutItem(
        text: Text(s.stopKeepingOffline),
        onPressed: () => _saveSelection(context, type, id, false),
      ),
    ],
  );
}

Future<void> _saveSelection(
  BuildContext context,
  CollectionType type,
  int id,
  bool enabled,
) async {
  final host = Navigator.of(context).context;
  final s = S.of(context);
  Flyout.of(context).close();
  String? failure;
  try {
    await setDownloadRule(type, id, enabled);
  } catch (error) {
    failure = error.toString();
  }
  if (!host.mounted) return;
  displayInfoBar(
    host,
    builder: (_, close) => InfoBar(
      title: Text(
        failure != null
            ? s.offlineSelectionFailed
            : enabled
            ? s.offlineSelectionSaved
            : s.offlineSelectionRemoved,
      ),
      content: Text(
        failure ??
            (enabled ? s.offlineSelectionQueued : s.offlineSelectionRetained),
      ),
      severity: failure == null
          ? InfoBarSeverity.success
          : InfoBarSeverity.error,
      onClose: close,
    ),
  );
}
