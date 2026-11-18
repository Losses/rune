import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../constants/configurations.dart';
import '../../constants/settings_manager.dart';
import '../router/navigation.dart';
import '../../providers/library_open_state.dart';
import '../../providers/request_capabilities.dart';
import '../api/set_download_rule.dart';
import '../api/request_failure.dart';
import '../l10n.dart';
import '../dialogs/information/error.dart';

/// The offline selection is kept in the open library's database, which a service library
/// does not have on this device. The hub says whether the open library has one.
bool supportsOfflineSelection(BuildContext context) {
  final record = context.read<LibraryOpenStateProvider>().openRecord;
  return record != null &&
      record.kind is! LibraryRecordKindSummaryRemoteControl &&
      capabilityForRequest(
            context,
            (capabilities) => capabilities.setDownloadRuleRequest,
            listen: false,
          )
          is RequestCapabilityAvailable;
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
  var deleteFiles = false;
  if (!enabled) {
    deleteFiles =
        await $settingsManager.getValue<bool>(kOfflineCleanupDefaultKey) ??
        true;
    if (!host.mounted) return;
    final result = await $showModal<bool>(
      host,
      (context, close) =>
          OfflineCleanupDialog(initialValue: deleteFiles, onClose: close),
    );
    final confirmed = result != null;
    deleteFiles = result ?? false;
    if (confirmed != true || !host.mounted) return;
  }
  try {
    await setDownloadRule(
      type,
      id,
      enabled,
      deleteUnselectedFiles: deleteFiles,
    );
  } on RequestFailure catch (error) {
    if (!host.mounted) return;
    await showErrorDialog(
      context: host,
      title: s.offlineSelectionFailed,
      errorMessage: requestFailureMessage(s, error),
    );
  } catch (error) {
    if (!host.mounted) return;
    await showErrorDialog(
      context: host,
      title: s.offlineSelectionFailed,
      errorMessage: requestErrorText(s, error),
    );
  }
}

class OfflineCleanupDialog extends StatefulWidget {
  const OfflineCleanupDialog({
    super.key,
    required this.initialValue,
    required this.onClose,
  });
  final bool initialValue;
  final void Function(bool?) onClose;

  @override
  State<OfflineCleanupDialog> createState() => _OfflineCleanupDialogState();
}

class _OfflineCleanupDialogState extends State<OfflineCleanupDialog> {
  late bool deleteFiles = widget.initialValue;

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return ContentDialog(
      title: Text(s.stopKeepingOffline),
      content: LayoutBuilder(
        builder: (context, constraints) => Checkbox(
          checked: deleteFiles,
          content: SizedBox(
            width: (constraints.maxWidth - 32).clamp(0, double.infinity),
            child: Text(s.offlineCleanupCheckbox),
          ),
          onChanged: (value) => setState(() => deleteFiles = value ?? false),
        ),
      ),
      actions: [
        Button(onPressed: () => widget.onClose(null), child: Text(s.cancel)),
        FilledButton(
          onPressed: () => widget.onClose(deleteFiles),
          child: Text(s.confirm),
        ),
      ],
    );
  }
}
