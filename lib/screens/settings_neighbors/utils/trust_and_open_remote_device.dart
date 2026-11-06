import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../bindings/bindings.dart';
import '../../../providers/library_path.dart';
import '../../../utils/api/add_trusted_server.dart';
import '../../../utils/api/register_device_on_server.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';

import 'remote_device_address.dart';
import 'show_server_status_dialog.dart';

/// Trusts the server with [fingerprint], registers this device on it, checks
/// the access status and stores the device in the library history.
///
/// Returns the stored library path, or null if any step failed or was
/// dismissed.
Future<String?> trustAndOpenRemoteDevice(
  BuildContext context, {
  required String fingerprint,
  required List<String> hosts,
  required String alias,
}) async {
  final s = S.of(context);
  final libraryPath = Provider.of<LibraryPathProvider>(context, listen: false);

  try {
    await addTrustedServer(fingerprint, hosts);
  } catch (e) {
    if (!context.mounted) return null;

    showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: e.toString(),
    );

    return null;
  }

  try {
    await registerDeviceOnServer(hosts);
  } catch (e) {
    if (!context.mounted) return null;

    showErrorDialog(
      context: context,
      title: s.unknownError,
      errorMessage: e.toString(),
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
  libraryPath.addLibraryPath(context, path, alias);

  return path;
}
