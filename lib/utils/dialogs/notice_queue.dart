import 'package:fluent_ui/fluent_ui.dart';

/// Shows the dialogs no page asked for, such as a failure no caller waits for
/// or a lost link, one at a time. They arrive from Rust at any moment, several
/// at once when a connection drops, and would otherwise pile up.
class NoticeQueue {
  final BuildContext? Function() _context;
  final List<(String, Future<void> Function(BuildContext))> _waiting = [];
  String? _shown;

  /// [context] gives the context the next dialog opens from, or none while
  /// there is nowhere to show it.
  NoticeQueue(this._context);

  /// Shows [dialog] once the dialogs before it are closed. A notice with the
  /// [message] of one that is shown or waiting says nothing new and is left
  /// out.
  void add(String message, Future<void> Function(BuildContext) dialog) {
    if (_shown == message || _waiting.any((notice) => notice.$1 == message)) {
      return;
    }
    _waiting.add((message, dialog));
    if (_shown == null) _showNext();
  }

  void _showNext() {
    if (_waiting.isEmpty) return;
    final context = _context();
    if (context == null || !context.mounted) {
      _waiting.clear();
      return;
    }
    final (message, dialog) = _waiting.removeAt(0);
    _shown = message;
    dialog(context).whenComplete(() {
      _shown = null;
      _showNext();
    });
  }
}
