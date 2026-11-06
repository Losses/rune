import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_path.dart';
import '../../../utils/api/close_library.dart';
import '../../../utils/dialogs/failed_to_initialize_library.dart';
import '../../../utils/dialogs/information/information.dart';
import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';
import '../../../utils/remote_connection_messages.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';

import '../utils/add_remote_device_form_controller.dart';
import '../utils/show_fingerprint_quiz_dialog.dart';
import '../utils/trust_and_open_remote_device.dart';

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
  late AddRemoteDeviceFormController controller;
  final _formKey = GlobalKey<FormState>();

  @override
  void initState() {
    super.initState();
    controller = AddRemoteDeviceFormController();
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }

  Future<void> _addConnection() async {
    final s = S.of(context);
    if (!(_formKey.currentState?.validate() ?? false)) return;
    final host = parseIpv4Host(controller.hostnameController.text)!;

    final libraryPath = Provider.of<LibraryPathProvider>(
      context,
      listen: false,
    );

    final quiz = await showFingerprintQuizDialog(context, host);
    if (quiz == null || !mounted) return;

    if (!quiz.correct) {
      showInformationDialog(
        context: context,
        title: s.pairingFailureTitle,
        subtitle: s.pairingFailureMessage,
      );
      return;
    }

    final path = await trustAndOpenRemoteDevice(
      context,
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
          controller: controller,
          formKey: _formKey,
          onSubmitted: _addConnection,
        ),
        actions: [
          FilledButton(onPressed: _addConnection, child: Text(s.addConnection)),
          Button(onPressed: () => widget.$close(null), child: Text(s.cancel)),
        ],
      ),
    );
  }
}
