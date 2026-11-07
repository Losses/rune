import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/api/add_trusted_server.dart';
import '../../../utils/api/register_device_on_server.dart';
import '../../../utils/api/request_failure.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';

import 'show_server_status_dialog.dart';

/// Trust a device whose certificate the user has verified, register this
/// device with it and show whether it accepted us. Pairing only: it does not
/// open or add any library.
Future<void> pairWithDevice(
  BuildContext context,
  String fingerprint,
  List<String> hosts,
) async {
  final s = S.of(context);
  try {
    await addTrustedServer(fingerprint, hosts);
    await registerDeviceOnServer(hosts);
  } catch (error) {
    if (!context.mounted) return;
    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(s, error),
    );
    return;
  }

  if (!context.mounted) return;
  final status = await showServerStatusDialog(context, hosts);
  if (status == null || !context.mounted) return;

  if (status.status == ClientStatus.blocked) {
    await showErrorDialog(
      context: context,
      title: s.clientBlockedTitle,
      errorMessage: s.clientBlockedMessage,
    );
  } else if (!status.success) {
    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: status.error,
    );
  }
}
