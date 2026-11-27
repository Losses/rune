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
) => $showModal<void>(
  context,
  (context, $close) => _RemoteReconnectingDialog(reconnecting, $close),
  barrierDismissible: true,
  dismissWithEsc: true,
);

class _RemoteReconnectingDialog extends StatefulWidget {
  final ValueListenable<String?> reconnecting;
  final void Function(void result) close;

  const _RemoteReconnectingDialog(this.reconnecting, this.close);

  @override
  State<_RemoteReconnectingDialog> createState() =>
      _RemoteReconnectingDialogState();
}

class _RemoteReconnectingDialogState extends State<_RemoteReconnectingDialog> {
  late String _alias = widget.reconnecting.value ?? '';

  @override
  void initState() {
    super.initState();
    widget.reconnecting.addListener(_changed);
    // The library can be back before this dialog's first frame, and nothing
    // is heard of it after that. The route leaves once it is built.
    if (widget.reconnecting.value == null) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && widget.reconnecting.value == null) _changed();
      });
    }
  }

  @override
  void dispose() {
    widget.reconnecting.removeListener(_changed);
    super.dispose();
  }

  void _changed() {
    final alias = widget.reconnecting.value;
    if (alias != null) {
      setState(() => _alias = alias);
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
              S.of(context).remoteLibraryReconnecting(_alias),
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
