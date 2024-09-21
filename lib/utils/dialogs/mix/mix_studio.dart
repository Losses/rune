import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/mix.pbserver.dart';

import './mix_studio_dialog.dart';

Future<MixWithoutCoverIds?> showMixStudioDialog(
  BuildContext context, {
  int? mixId,
}) async {
  return await showDialog<MixWithoutCoverIds?>(
    context: context,
    builder: (context) => MixStudioDialog(mixId: mixId),
  );
}
