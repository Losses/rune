import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/playback_controller/queue.dart';
import '../../utils/l10n.dart';
import 'play_on/play_on_dialog.dart';

import '../../widgets/rune_clickable.dart';
import '../router/navigation.dart';

typedef PlayOnDialogOpener = Future<void> Function(BuildContext context);

Future<void> showPlayQueueDialog(
  BuildContext context, {
  int? mixId,
  (String, String)? operator,
  PlayOnDialogOpener playOnDialog = showPlayOnDialog,
}) async {
  final parentContext = context;
  return await $showModal<void>(
    context,
    (_, $close) => ContentDialog(
      constraints: const BoxConstraints(maxWidth: 420, maxHeight: 600),
      title: Row(
        children: [
          const SizedBox(width: 12),
          Text(S.of(context).queue),
          Expanded(child: Container()),
          Tooltip(
              message: S.of(context).playOn,
              child: RuneClickable(
                key: const Key('queue-play-on'),
                child: const Icon(Symbols.cast, size: 24),
                onPressed: () => playOnDialog(parentContext),
              ),
          ),
          const SizedBox(width: 8),
          RuneClickable(
            child: const Icon(Symbols.close, size: 24),
            onPressed: () => $close(null),
          ),
        ],
      ),
      content: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 240),
        child: const Align(alignment: Alignment.topCenter, child: Queue()),
      ),
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
