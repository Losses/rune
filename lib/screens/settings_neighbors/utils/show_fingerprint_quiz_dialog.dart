import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/fingerprint_quiz_dialog.dart';

/// Shows the fingerprint quiz for the device at [host].
///
/// Returns whether the user picked the right fingerprint, together with the
/// fingerprint of the certificate the device served, or null when the user
/// left the quiz without answering.
Future<FingerprintQuizResult?> showFingerprintQuizDialog(
  BuildContext context,
  String host,
) {
  return $showModal<FingerprintQuizResult?>(
    context,
    (context, $close) => FingerprintQuizDialog(
      host: host,
      $close: $close,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
