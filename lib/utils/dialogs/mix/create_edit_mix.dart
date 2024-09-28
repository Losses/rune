import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/mix.pb.dart';
import '../../../utils/dialogs/mix/create_edit_mix_dialog.dart';

Future<Mix?> showCreateEditMixDialog(
  BuildContext context, {
  int? mixId,
  (String, String)? operator,
}) async {
  return await showDialog<Mix?>(
    context: context,
    builder: (context) => CreateEditMixDialog(mixId: mixId, operator: operator),
  );
}
