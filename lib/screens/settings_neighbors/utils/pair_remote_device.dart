import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/library_path.dart';
import '../../../utils/api/add_trusted_server.dart';
import '../../../utils/api/register_device_on_server.dart';
import '../../../utils/api/request_failure.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';

import 'show_server_status_dialog.dart';

/// Pairs with a remote device after its fingerprint quiz: trusts the server
/// with [fingerprint], registers this device on it, checks the access status
/// and stores the device in the library history. It does not open the library.
///
/// A wrong quiz answer ([correct] is false) is reported here too, so that
/// every caller shows the same failure.
///
/// Returns the stored library path, or null if any step failed or was
/// dismissed.
Future<String?> pairRemoteDevice(
  BuildContext context, {
  required bool correct,
  required String fingerprint,
  required List<String> hosts,
  required String alias,
}) async {
  final s = S.of(context);
  final libraryPath = Provider.of<LibraryPathProvider>(context, listen: false);

  if (!correct) {
    await showInformationDialog(
      context: context,
      title: s.pairingFailureTitle,
      subtitle: s.pairingFailureMessage,
    );
    return null;
  }

  try {
    await addTrustedServer(fingerprint, hosts);
  } catch (e) {
    if (!context.mounted) return null;

    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(s, e),
    );

    return null;
  }

  try {
    await registerDeviceOnServer(hosts);
  } catch (e) {
    if (!context.mounted) return null;

    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: requestErrorText(s, e),
    );

    return null;
  }

  if (!context.mounted) return null;
  final serverStatus = await showServerStatusDialog(context, hosts);

  if (serverStatus == null) {
    return null;
  }

  if (!context.mounted) return null;
  if (!serverStatus.success) {
    await showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: serverStatus.error,
    );

    return null;
  } else if (serverStatus.status == ClientStatus.blocked) {
    await showErrorDialog(
      context: context,
      title: s.clientBlockedTitle,
      errorMessage: s.clientBlockedMessage,
    );

    return null;
  }

  if (!context.mounted) return null;
  final path = remoteLibraryPath(hosts);
  // Awaited because setLibraryPath stores the history as well, and the two
  // writes must not overlap.
  await libraryPath.addLibraryPath(context, path, alias);

  return path;
}
