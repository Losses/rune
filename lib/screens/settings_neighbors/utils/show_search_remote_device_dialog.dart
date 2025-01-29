import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/search_remote_device_dialog.dart';

void showSearchRemoteDeviceDialog(BuildContext context) {
  $showModal<void>(
    context,
    (context, $close) => SearchRemoteDeviceDialog(
      $close: $close,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
