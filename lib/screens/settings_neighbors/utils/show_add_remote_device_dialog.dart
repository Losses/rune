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
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
