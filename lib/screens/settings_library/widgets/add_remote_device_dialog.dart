import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../messages/all.dart';

import '../utils/add_remote_device_form_controller.dart';

import 'add_remote_device_form.dart';

class AddRemoteDeviceDialog extends StatefulWidget {
  final void Function(LoginRequestItem?) $close;

  const AddRemoteDeviceDialog({
    super.key,
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

  _addConnection() {}

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return NoShortcuts(
      ContentDialog(
        title: Text(s.addConnection),
        content: AddRemoteDeviceForm(
          controller: controller,
        ),
        actions: [
          FilledButton(
            onPressed: _addConnection,
            child: Text(s.addConnection),
          ),
          Button(
            onPressed: () => widget.$close(null),
            child: Text(s.cancel),
          ),
        ],
      ),
    );
  }
}
