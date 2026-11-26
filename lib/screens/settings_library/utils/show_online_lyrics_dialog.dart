import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/library_registry.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';

/// Says where the lyrics of a library's songs come from and what looking them
/// up sends, and lets the user turn the lookups on or off. [enabled] is what
/// the library does now, and [onChanged] sends the user's choice, throwing when
/// it could not be kept.
Future<void> showOnlineLyricsDialog(
  BuildContext context, {
  required bool enabled,
  required Future<void> Function(bool enabled) onChanged,
}) {
  return $showModal<void>(
    context,
    (context, $close) => _OnlineLyricsDialog(
      enabled: enabled,
      onChanged: onChanged,
      close: () => $close(null),
    ),
    dismissWithEsc: true,
    barrierDismissible: true,
  );
}

class _OnlineLyricsDialog extends StatefulWidget {
  const _OnlineLyricsDialog({
    required this.enabled,
    required this.onChanged,
    required this.close,
  });

  final bool enabled;
  final Future<void> Function(bool enabled) onChanged;
  final VoidCallback close;

  @override
  State<_OnlineLyricsDialog> createState() => _OnlineLyricsDialogState();
}

class _OnlineLyricsDialogState extends State<_OnlineLyricsDialog> {
  late bool _enabled = widget.enabled;

  /// Whether a choice is on its way to the hub, during which the switch waits.
  bool _sending = false;

  Future<void> _set(bool enabled) async {
    final before = _enabled;
    setState(() {
      _enabled = enabled;
      _sending = true;
    });
    Object? error;
    try {
      await widget.onChanged(enabled);
    } catch (exception) {
      error = exception;
    }
    if (!mounted) return;
    setState(() {
      _sending = false;
      if (error != null) _enabled = before;
    });
    if (error != null) {
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: libraryFailureMessage(S.of(context), error),
      );
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return ContentDialog(
      title: Column(children: [const SizedBox(height: 8), Text(s.lyrics)]),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(s.onlineLyricsNotice, style: const TextStyle(height: 1.4)),
          const SizedBox(height: 12),
          ToggleSwitch(
            checked: _enabled,
            // The switch is at the right edge with its label on the left, as
            // the mix dialog has them. The label wraps in a narrow dialog
            // instead of running past its edge.
            leadingContent: true,
            content: Expanded(child: Text(s.lookUpLyricsOnLrclib)),
            onChanged: _sending ? null : _set,
          ),
        ],
      ),
      actions: [Button(onPressed: widget.close, child: Text(s.close))],
    );
  }
}
