import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';
import '../utils/add_remote_device_form_controller.dart';

class AddRemoteDeviceForm extends StatelessWidget {
  final AddRemoteDeviceFormController controller;
  final GlobalKey<FormState>? formKey;
  final VoidCallback? onSubmitted;

  const AddRemoteDeviceForm({
    super.key,
    required this.controller,
    this.formKey,
    this.onSubmitted,
  });

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return Form(
      key: formKey,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          const SizedBox(height: 16),
          InfoLabel(
            label: s.ipAddress,
            child: TextFormBox(
              controller: controller.hostnameController,
              placeholder: '192.168.1.20',
              autofocus: true,
              autovalidateMode: AutovalidateMode.onUserInteraction,
              validator: (value) => parseIpv4Host(value ?? '') == null
                  ? s.invalidIpv4Address
                  : null,
              onFieldSubmitted: (_) => onSubmitted?.call(),
            ),
          ),
        ],
      ),
    );
  }
}
