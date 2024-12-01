import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

import 'widget/register_dialog.dart';

void showRegisterDialog(BuildContext context) {
  $showModal<void>(
    context,
    (context, $close) => RegisterDialog(
      $close: $close,
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
