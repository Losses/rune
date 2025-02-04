import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';

import '../widgets/review_connection_dialog.dart';

void showReviewConnectionDialog(BuildContext context, String fingerprint) {
  $showModal<void>(
    context,
    (context, $close) => ReviewConnectionDialog(
      $close: $close,
      fingerprint: fingerprint,
    ),
    barrierDismissible: false,
    dismissWithEsc: false,
  );
}
