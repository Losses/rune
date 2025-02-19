import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/fingerprint_quiz_dialog.dart';

void showFingerprintQuizDialog(BuildContext context, String host) {
  $showModal<void>(
    context,
    (context, $close) => FingerprintQuizDialog(
      host: host,
      $close: $close,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
