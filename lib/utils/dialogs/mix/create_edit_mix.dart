import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/mix.pb.dart';
import '../../../utils/dialogs/mix/create_edit_mix_dialog.dart';

Future<MixWithoutCoverIds?> showCreateEditMixDialog(BuildContext context,
    {int? mixId}) async {
  return await showDialog<MixWithoutCoverIds?>(
    context: context,
    builder: (context) => CreateEditMixDialog(mixId: mixId),
  );
}
