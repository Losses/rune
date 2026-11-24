import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/edit_device_information_setting_dialog.dart';

void showEditDeviceInformationSettingDialog(BuildContext context) {
  $showModal<void>(
    context,
    (context, $close) => EditDeviceInformationSettingDialog(
      $close: $close,
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
