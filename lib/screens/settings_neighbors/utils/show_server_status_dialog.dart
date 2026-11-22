import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/api/check_device_on_server.dart';
import '../../../utils/router/navigation.dart';

import '../widgets/server_status_dialog.dart';

Future<CheckDeviceOnServerResponse?> showServerStatusDialog(
  BuildContext context,
  List<String> hosts,
  String fingerprint, {
  DeviceStatusCheck check = checkDeviceOnServer,
}) async {
  return await $showModal<CheckDeviceOnServerResponse?>(
    context,
    (context, $close) => ServerStatusDialog(
      hosts: hosts,
      fingerprint: fingerprint,
      close: $close,
      check: check,
    ),
    barrierDismissible: false,
    dismissWithEsc: true,
  );
}
