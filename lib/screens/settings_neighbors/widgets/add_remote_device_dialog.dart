import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/api/add_trusted_server.dart';
import '../../../utils/api/register_device_on_server.dart';
import '../../../utils/dialogs/information/error.dart';
import '../../../utils/api/request_failure.dart';
import '../utils/show_fingerprint_quiz_dialog.dart';
import '../../../utils/l10n.dart';
import '../../../utils/router/navigation.dart';
import '../../../widgets/no_shortcuts.dart';

import '../utils/add_remote_device_form_controller.dart';

import 'add_remote_device_form.dart';

class AddRemoteDeviceDialog extends StatefulWidget {
  final bool navigateIfFailed;
  final void Function(bool?) $close;

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

  _addConnection() async {
    final url = Uri.parse(controller.toWebSocketUrl());
    final host = '${url.host}:${url.port}';
    final answer = await showFingerprintQuizDialog(context, host);
    if (answer == null || !answer.correct || !mounted) return;
    try {
      await addTrustedServer(answer.fingerprint, [host]);
      await registerDeviceOnServer([host], answer.fingerprint);
      widget.$close(true);
    } catch (error) {
      if (!mounted) return;
      await showErrorDialog(
        context: context,
        title: S.of(context).unknownError,
        errorMessage: requestErrorText(S.of(context), error),
      );
      if (widget.navigateIfFailed) $$replace('/');
    }
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Column(children: [SizedBox(height: 8), Text(s.addConnection)]),
        content: AddRemoteDeviceForm(controller: controller),
        actions: [
          FilledButton(onPressed: _addConnection, child: Text(s.addConnection)),
          Button(onPressed: () => widget.$close(null), child: Text(s.cancel)),
        ],
      ),
    );
  }
}
