import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';
import '../../../utils/router/navigation.dart';
import '../widgets/add_remote_device_dialog.dart';

void showAddRemoteDeviceDialog(BuildContext context) {
  $showModal<LoginRequestItem>(
    context,
    (context, $close) => AddRemoteDeviceDialog($close: $close),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
