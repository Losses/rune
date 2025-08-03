import 'package:fluent_ui/fluent_ui.dart';

import '../../../bindings/bindings.dart';

import '../../router/navigation.dart';

import './mix_studio_dialog.dart';

Future<Mix?> showMixStudioDialog(
  BuildContext context, {
  int? mixId,
}) async {
  return await $showModal<Mix?>(
    context,
    (context, $close) => MixStudioDialog(
      mixId: mixId,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
