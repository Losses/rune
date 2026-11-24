import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../bindings/bindings.dart';

import '../widgets/search_remote_device_dialog.dart';
import 'show_fingerprint_quiz_dialog.dart';

Future<(DiscoveredDeviceMessage, bool?)?> showSearchRemoteDeviceDialog(
  BuildContext context,
) async {
  ModalRoute<DiscoveredDeviceMessage>? searchRoute;
  final device = await $showModal<DiscoveredDeviceMessage>(
    context,
    (dialogContext, $close) {
      searchRoute ??= ModalRoute.of<DiscoveredDeviceMessage>(dialogContext);
      return SearchRemoteDeviceDialog($close: $close);
    },
    barrierDismissible: false,
    dismissWithEsc: false,
  );
  // Let the search leave the navigator before opening the next dialog.
  await searchRoute?.completed;
  if (device == null || !context.mounted) return null;

  final answer = await showFingerprintQuizDialog(
    context,
    device.addresses.first,
  );
  if (!context.mounted) return null;

  // The certificate the user checked must be the one the device announced.
  return (
    device,
    answer == null
        ? null
        : answer.correct && answer.fingerprint == device.fingerprint,
  );
}
