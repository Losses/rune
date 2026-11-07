import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../utils/add_remote_device_form_controller.dart';

class AddRemoteDeviceForm extends StatelessWidget {
  final AddRemoteDeviceFormController controller;

  const AddRemoteDeviceForm({
    super.key,
    required this.controller,
  });

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const SizedBox(height: 16),
        InfoLabel(
          label: s.hostname,
          child: TextBox(controller: controller.hostnameController),
        ),
        const SizedBox(height: 16),
        InfoLabel(
          label: s.port,
          child: TextBox(controller: controller.portController),
        ),
        const SizedBox(height: 20),
        ToggleSwitch(
          checked: controller.securedController.isChecked,
          onChanged: (v) => controller.securedController.isChecked = v,
          content: Text(s.secured),
        )
      ],
    );
  }
}
