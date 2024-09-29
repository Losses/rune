import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller/playlist.dart';
import '../../messages/mix.pb.dart';

Future<Mix?> showPlayQueueDialog(
  BuildContext context, {
  int? mixId,
  (String, String)? operator,
}) async {
  return await showDialog<Mix?>(
    context: context,
    builder: (context) => const Playlist(),
  );
}
