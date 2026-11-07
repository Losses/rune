import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/add_remote_device_dialog.dart';

void showAddRemoteDeviceDialog(bool navigateIfFailed, BuildContext context) {
  $showModal<void>(
    context,
    (context, $close) => AddRemoteDeviceDialog(
      navigateIfFailed: navigateIfFailed,
      $close: $close,
    ),
    // Pairing and opening the device take a while once Add is pressed, and must
    // not be cut off half way, so the dialog closes only through its buttons.
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
