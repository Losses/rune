import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/update_client_status.dart';
import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../bindings/bindings.dart';

import '../widgets/review_connection_dialog.dart';

Future<void> showReviewConnectionDialog(
  BuildContext context,
  ClientSummary clientSummary,
) async {
  final s = S.of(context);
  final result = await $showModal<ClientStatus>(
    context,
    (context, $close) => ReviewConnectionDialog(
      $close: $close,
      clientSummary: clientSummary,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );

  if (result != null) {
    try {
      await updateClientStatus(
        clientSummary.fingerprint,
        result,
      );
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
