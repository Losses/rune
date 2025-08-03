import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../utils/dialogs/mix/create_edit_mix_dialog.dart';
import '../../../bindings/bindings.dart';

Future<Mix?> showCreateEditMixDialog(
  BuildContext context, 
  String? defaultTitle, {
  int? mixId,
  (String, String)? operator,
}) async {
  return await $showModal<Mix?>(
    context,
    (context, $close) => CreateEditMixDialog(
      mixId: mixId,
      defaultTitle: defaultTitle,
      operator: operator,
      $close: $close,
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}
