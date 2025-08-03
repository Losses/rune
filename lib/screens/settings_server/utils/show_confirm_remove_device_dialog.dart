import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/remove_trusted_client.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/dialogs/information/confirm.dart';
import '../../../bindings/bindings.dart';

Future<void> showConfirmRemoveDeviceDialog(
  BuildContext context,
  ClientSummary client,
) async {
  final s = S.of(context);
  final result = await showConfirmDialog(
    context: context,
    title: s.removeTrust,
    subtitle: s.removeTrustSubtitle,
    yesLabel: s.remove,
    noLabel: s.cancel,
  );

  if (result == true) {
    try {
      await removeTrustedClient(client.fingerprint);
    } catch (e) {
      if (!context.mounted) return;
      showErrorDialog(
        context: context,
        title: s.unknownError,
        errorMessage: e.toString(),
      );
    }
  }
}
