import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/all.dart';

import '../../router/navigation.dart';

import 'scrobble_login_dialog.dart';

void showScrobbleLoginDialog(BuildContext context, String serviceName) {
  $showModal<LoginRequestItem>(
    context,
    (context, $close) => ScrobbleLoginDialog(
      $close: $close,
      serviceName: serviceName,
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
