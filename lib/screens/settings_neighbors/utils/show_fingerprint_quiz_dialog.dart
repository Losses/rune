import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/api/fetch_server_certificate.dart';
import '../../../utils/router/navigation.dart';

import '../widgets/fingerprint_quiz_dialog.dart';

Future<FingerprintAnswer?> showFingerprintQuizDialog(
  BuildContext context,
  String host, {
  FingerprintLoader loadFingerprint = fetchServerCertificate,
}) {
  return $showModal<FingerprintAnswer?>(
    context,
    (context, $close) => FingerprintQuizDialog(
      host: host,
      $close: $close,
      loadFingerprint: loadFingerprint,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
