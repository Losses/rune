import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';

import '../utils/add_remote_device_form_controller.dart';

import 'add_remote_device_form.dart';

class AddRemoteDeviceDialog extends StatefulWidget {
  final void Function(String?) $close;

  const AddRemoteDeviceDialog({super.key, required this.$close});

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

  /// Close with the `host[:port]` the user entered; the caller pairs.
  void _addConnection() {
    final uri = Uri.parse(controller.toWebSocketUrl());
    final hostName = uri.host.contains(':') ? '[${uri.host}]' : uri.host;
    widget.$close(uri.hasPort ? '$hostName:${uri.port}' : hostName);
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
