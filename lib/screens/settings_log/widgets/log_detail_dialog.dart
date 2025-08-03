import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/l10n.dart';
import '../../../bindings/bindings.dart';

class LogDetailDialog extends StatefulWidget {
  const LogDetailDialog({
    super.key,
    required this.logs,
    required this.initialIndex,
    required this.onClose,
  });

  final List<LogDetail> logs;
  final int initialIndex;
  final VoidCallback onClose;

  @override
  State<LogDetailDialog> createState() => _LogDetailDialogState();
}

class _LogDetailDialogState extends State<LogDetailDialog> {
  late int currentIndex;

  @override
  void initState() {
    super.initState();
    currentIndex = widget.initialIndex;
  }

  void _navigateTo(int index) {
    if (index >= 0 && index < widget.logs.length) {
      setState(() {
        currentIndex = index;
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final log = widget.logs[currentIndex];

    return ContentDialog(
      title: Text(log.level),
      constraints: const BoxConstraints(maxHeight: 320, maxWidth: 520),
      content: Column(
        key: ValueKey(currentIndex),
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(log.domain),
          const SizedBox(height: 4),
          Text(
            DateTime.fromMillisecondsSinceEpoch(log.date.toInt() * 1000)
                .toString(),
          ),
          const SizedBox(height: 8),
          Expanded(
            child: SingleChildScrollView(
              child: SelectableText(
                log.detail,
                style: const TextStyle(height: 1.25),
              ),
            ),
          ),
        ],
      ),
      actions: [
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceBetween,
          children: [
            Row(
              children: [
                Button(
                  onPressed: currentIndex > 0
                      ? () => _navigateTo(currentIndex - 1)
                      : null,
                  child: Row(
                    children: [
                      const Icon(Symbols.arrow_back),
                      const SizedBox(width: 4),
                      Text(S.of(context).previous),
                    ],
                  ),
                ),
                const SizedBox(width: 8),
                Button(
                  onPressed: currentIndex < widget.logs.length - 1
                      ? () => _navigateTo(currentIndex + 1)
                      : null,
                  child: Row(
                    children: [
                      Text(S.of(context).next),
                      const SizedBox(width: 4),
                      const Icon(Symbols.arrow_forward),
                    ],
                  ),
                ),
              ],
            ),
            FilledButton(
              onPressed: widget.onClose,
              child: Text(S.of(context).close),
            ),
          ],
        ),
      ],
    );
  }
}
