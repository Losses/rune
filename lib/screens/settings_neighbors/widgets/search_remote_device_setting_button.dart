import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../utils/l10n.dart';
import '../../../utils/api/add_trusted_server.dart';
import '../../../utils/api/register_device_on_server.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../utils/discovery_url.dart';
import '../../../widgets/settings/settings_button.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/library_path.dart';

import '../utils/show_search_remote_device_dialog.dart';
import '../utils/show_server_status_dialog.dart';

class SearchRemoteDeviceSettingButton extends StatelessWidget {
  const SearchRemoteDeviceSettingButton({
    super.key,
    required this.tryClose,
    required this.navigateIfFailed,
  });

  final bool tryClose;
  final bool navigateIfFailed;

  @override
  Widget build(BuildContext context) {
    final libraryPath =
        Provider.of<LibraryPathProvider>(context, listen: false);
    final s = S.of(context);

    return SettingsButton(
      icon: Symbols.search,
      title: s.searchNeighbors,
      subtitle: s.searchNeighborsSubtitle,
      onPressed: () async {
        final response = await showSearchRemoteDeviceDialog(context);
        if (response == null) return;

        final (device, correct) = response;

        if (correct == null) return;

        if (!correct) {
          if (!context.mounted) return;
          showInformationDialog(
            context: context,
            title: s.pairingFailureTitle,
            subtitle: s.pairingFailureMessage,
          );

          return;
        }

        try {
          await addTrustedServer(device.fingerprint, device.ips);
        } catch (e) {
          if (!context.mounted) return;

          showErrorDialog(
            context: context,
            title: s.unknownError,
            errorMessage: e.toString(),
          );
        }

        try {
          await registerDeviceOnServer(device.ips);
        } catch (e) {
          if (!context.mounted) return;

          showErrorDialog(
            context: context,
            title: s.unknownError,
            errorMessage: e.toString(),
          );

          return;
        }

        if (!context.mounted) return;
        final serverStatus = await showServerStatusDialog(context, device.ips);

        if (serverStatus == null) {
          return;
        }

        if (!context.mounted) return;
        if (!serverStatus.success) {
          await showErrorDialog(
            context: context,
            title: s.unknownError,
            errorMessage: serverStatus.error,
          );

          return;
        } else if (serverStatus.status == ClientStatus.blocked) {
          await showErrorDialog(
            context: context,
            title: s.clientBlockedTitle,
            errorMessage: s.clientBlockedMessage,
          );

          return;
        }

        if (!context.mounted) return;
        libraryPath.addLibraryPath(
          context,
          '@RR|${encodeRnSrvUrl(device.ips)}',
          device.alias,
        );
      },
    );
  }
}
