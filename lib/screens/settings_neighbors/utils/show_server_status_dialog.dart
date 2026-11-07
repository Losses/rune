import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';
import '../../../utils/router/navigation.dart';

import '../widgets/server_status_dialog.dart';

Future<CheckDeviceOnServerResponse?> showServerStatusDialog(
  BuildContext context,
  List<String> hosts,
) async {
  return await $showModal<CheckDeviceOnServerResponse?>(
    context,
    (context, $close) => ServerStatusDialog(
      hosts: hosts,
      close: $close,
    ),
    barrierDismissible: false,
    dismissWithEsc: true,
  );
}
