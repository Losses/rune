import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/dialogs/information/information.dart';
import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';

import '../widgets/add_remote_device_dialog.dart';

import 'pair_with_device.dart';
import 'show_fingerprint_quiz_dialog.dart';

/// Ask for an address, let the user verify the device's certificate, then pair.
Future<void> showAddRemoteDeviceDialog(BuildContext context) async {
  final host = await $showModal<String?>(
    context,
    (context, $close) => AddRemoteDeviceDialog($close: $close),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
  if (host == null || !context.mounted) return;

  final answer = await showFingerprintQuizDialog(context, host);
  if (answer == null || !context.mounted) return;
  if (!answer.correct) {
    final s = S.of(context);
    await showInformationDialog(
      context: context,
      title: s.pairingFailureTitle,
      subtitle: s.pairingFailureMessage,
    );
    return;
  }
  await pairWithDevice(context, answer.fingerprint, [host]);
}
