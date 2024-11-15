import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/playback_controller/queue.dart';
import '../../generated/l10n.dart';

import '../router/navigation.dart';

Future<void> showPlayQueueDialog(
  BuildContext context, {
  int? mixId,
  (String, String)? operator,
}) async {
  return await $showModal<void>(
    context,
    (context, $close) => ContentDialog(
      constraints: const BoxConstraints(maxWidth: 420, maxHeight: 600),
      title: Row(
        children: [
          const SizedBox(width: 12),
          Text(S.of(context).queue),
          Expanded(
            child: Container(),
          ),
          IconButton(
            icon: const Icon(
              Symbols.close,
              size: 24,
            ),
            onPressed: () => $close(null),
          ),
        ],
      ),
      content: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: 240),
        child: const Align(
          alignment: Alignment.topCenter,
          child: Queue(),
        ),
      ),
    ),
    barrierDismissible: true,
    dismissWithEsc: true,
  );
}
