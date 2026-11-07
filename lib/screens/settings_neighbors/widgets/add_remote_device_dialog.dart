import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_path.dart';
import '../../../utils/api/close_library.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';
import '../../../utils/remote_connection_messages.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';

import '../utils/show_fingerprint_quiz_dialog.dart';
import '../utils/pair_remote_device.dart';

import 'add_remote_device_form.dart';

class AddRemoteDeviceDialog extends StatefulWidget {
  final bool navigateIfFailed;
  final void Function(void) $close;

  const AddRemoteDeviceDialog({
    super.key,
    required this.navigateIfFailed,
    required this.$close,
  });

  @override
  AddRemoteDeviceDialogState createState() => AddRemoteDeviceDialogState();
}

class AddRemoteDeviceDialogState extends State<AddRemoteDeviceDialog> {
  final TextEditingController hostController = TextEditingController();
  final _formKey = GlobalKey<FormState>();
  // Whether Add was pressed once: from then on the field validates as typed.
  bool _tried = false;
  // The pairing and the switch of the library take long, and run only once.
  bool isLoading = false;

  @override
  void dispose() {
    hostController.dispose();
    super.dispose();
  }

  Future<void> _addConnection() async {
    if (isLoading) return;
    if (!_tried) setState(() => _tried = true);
    if (!(_formKey.currentState?.validate() ?? false)) return;
    final host = parseIpv4Host(hostController.text)!;

    setState(() => isLoading = true);
    try {
      await _pairAndOpen(host);
    } finally {
      if (mounted) setState(() => isLoading = false);
    }
  }

  Future<void> _pairAndOpen(String host) async {
    final libraryPath = Provider.of<LibraryPathProvider>(
      context,
      listen: false,
    );

    final quiz = await showFingerprintQuizDialog(context, host);
    if (quiz == null || !mounted) return;

    final path = await pairRemoteDevice(
      context,
      correct: quiz.correct,
      fingerprint: quiz.fingerprint,
      hosts: [host],
      alias: host,
    );
    if (path == null || !mounted) return;

    await closeLibrary(context);

    if (!mounted) return;

    final (switched, cancelled, _, errorKind) =
        await libraryPath.setLibraryPath(context, path, null);

    widget.$close(null);
    if (!switched && !cancelled) {
      if (!context.mounted) return;
      await showFailedToInitializeLibrary(
        context,
        remoteOpenErrorMessage(S.of(context), errorKind),
      );
      if (widget.navigateIfFailed) {
        $$replace('/');
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(s.addConnection)]),
        content: AddRemoteDeviceForm(
          controller: hostController,
          formKey: _formKey,
          validateAsTyped: _tried,
          enabled: !isLoading,
          onSubmitted: _addConnection,
        ),
        actions: [
          FilledButton(
            onPressed: isLoading ? null : _addConnection,
            child: isLoading
                ? const SizedBox(
                    width: 16,
                    height: 16,
                    child: ProgressRing(strokeWidth: 2),
                  )
                : Text(s.addConnection),
          ),
          Button(
            onPressed: isLoading ? null : () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
