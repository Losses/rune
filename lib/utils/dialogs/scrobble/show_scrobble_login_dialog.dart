import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';

import '../../router/navigation.dart';

import 'scrobble_login_dialog.dart';

void showScrobbleLoginDialog(BuildContext context, String serviceName, String title) {
  $showModal<LoginRequestItem>(
    context,
    (context, $close) => ScrobbleLoginDialog(
      $close: $close,
      title: title,
      serviceName: serviceName,
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
