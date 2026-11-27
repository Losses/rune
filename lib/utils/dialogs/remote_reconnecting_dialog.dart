import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';

import '../l10n.dart';
import '../router/navigation.dart';

/// Tells the user that the remote library named by [reconnecting] is being
/// reconnected. The dialog closes by itself once [reconnecting] holds no name,
/// and the user can close it before that.
Future<void> showRemoteReconnectingDialog(
  BuildContext context,
  ValueListenable<String?> reconnecting,
) => _showRetryDialog(
  context,
  reconnecting,
  (context, alias) => S.of(context).remoteLibraryReconnecting(alias),
);

/// Shows a dismissible retry notice for the open replica until it recovers.
Future<void> showPeerMetadataRetryDialog(
  BuildContext context,
  ValueListenable<String?> retrying,
) => _showRetryDialog(
  context,
  retrying,
  (context, _) => S.of(context).peerMetadataSyncRetrying,
);

Future<void> _showRetryDialog(
  BuildContext context,
  ValueListenable<String?> retrying,
  String Function(BuildContext, String) message,
) => $showModal<void>(
  context,
  (context, $close) => _RetryDialog(retrying, message, $close),
  barrierDismissible: true,
  dismissWithEsc: true,
);

class _RetryDialog extends StatefulWidget {
  final ValueListenable<String?> retrying;
  final String Function(BuildContext, String) message;
  final void Function(void result) close;

  const _RetryDialog(this.retrying, this.message, this.close);

  @override
  State<_RetryDialog> createState() => _RetryDialogState();
}

class _RetryDialogState extends State<_RetryDialog> {
  late String _item = widget.retrying.value ?? '';

  @override
  void initState() {
    super.initState();
    widget.retrying.addListener(_changed);
    // The library can be back before this dialog's first frame, and nothing
    // is heard of it after that. The route leaves once it is built.
    if (widget.retrying.value == null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && widget.retrying.value == null) _changed();
      });
    }
  }

  @override
  void dispose() {
    widget.retrying.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    final item = widget.retrying.value;
    if (item != null) {
      setState(() => _item = item);
      return;
    }
    final route = ModalRoute.of(context);
    if (route == null) return;
    if (route.isCurrent) {
      widget.close(null);
    } else {
      // Another dialog is open over this one, which leaves from under it.
      Navigator.of(context).removeRoute(route);
    }
  }

  @override
  Widget build(BuildContext context) {
    return ContentDialog(
      content: Row(
        children: [
          const SizedBox(width: 20, height: 20, child: ProgressRing()),
          const SizedBox(width: 16),
          Expanded(
            child: Text(
              widget.message(context, _item),
              style: const TextStyle(height: 1.4),
            ),
          ),
        ],
      ),
      actions: [
        Button(
          child: Text(S.of(context).close),
          onPressed: () => widget.close(null),
        ),
      ],
    );
  }
}
