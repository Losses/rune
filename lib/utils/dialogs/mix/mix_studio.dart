import 'package:fluent_ui/fluent_ui.dart';

import '../../../messages/playlist.pb.dart';

import './mix_studio_dialog.dart';

Future<PlaylistWithoutCoverIds?> showMixStudioDialog(
  BuildContext context, {
  int? mixId,
}) async {
  return await showDialog<PlaylistWithoutCoverIds?>(
    context: context,
    builder: (context) => MixStudioDialog(mixId: mixId),
  );
}
