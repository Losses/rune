import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/add_remote_device_dialog.dart';

Future<bool?> showAddRemoteDeviceDialog(
  bool navigateIfFailed,
  BuildContext context,
) {
  return $showModal<bool>(
    context,
    (context, $close) => AddRemoteDeviceDialog(
      navigateIfFailed: navigateIfFailed,
      $close: $close,
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
