import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_path.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';

import '../widgets/search_remote_device_dialog.dart';

void showSearchRemoteDeviceDialog(BuildContext context) {
  final libraryPath = Provider.of<LibraryPathProvider>(context, listen: false);

  $showModal<void>(
    context,
    (context, $close) => SearchRemoteDeviceDialog(
      $close: $close,
      onAnswered: (device, result) {
        if (result == false) {
          final s = S.of(context);

          showInformationDialog(
            context: context,
            title: s.pairingFailureTitle,
            subtitle: s.pairingFailureMessage,
          );
        } else if (result == true) {
          libraryPath.addLibraryPath(
            context,
            '@RR|${encodeRnSrvUrl(device.ips)}',
            device.alias,
          );
        }
      },
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
