import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/playback_controller/queue.dart';

Future<void> showPlayQueueDialog(
  BuildContext context, {
  int? mixId,
  (String, String)? operator,
}) async {
  return await showDialog<void>(
    barrierDismissible: true,
    dismissWithEsc: true,
    context: context,
    builder: (context) => ContentDialog(
      constraints: const BoxConstraints(maxWidth: 420, maxHeight: 600),
      title: Row(
        children: [
          const SizedBox(width: 12),
          const Text('Queue'),
          Expanded(
            child: Container(),
          ),
          IconButton(
            icon: const Icon(
              Symbols.close,
              size: 24,
            ),
            onPressed: () => Navigator.pop(context),
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
  );
}
