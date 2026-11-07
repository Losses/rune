import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../bindings/bindings.dart';

import '../widgets/search_remote_device_dialog.dart';

Future<(DiscoveredDeviceMessage, bool?)?> showSearchRemoteDeviceDialog(
    BuildContext context) async {
  final Completer<(DiscoveredDeviceMessage, bool?)?> completer = Completer();

  await $showModal<(DiscoveredDeviceMessage, bool?)>(
    context,
    (context, $close) => SearchRemoteDeviceDialog(
      $close: $close,
      completer: completer,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );

  return completer.future;
}
