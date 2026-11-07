import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/discovery_url.dart';
import '../../../utils/l10n.dart';

class AddRemoteDeviceForm extends StatelessWidget {
  final TextEditingController controller;
  final GlobalKey<FormState>? formKey;
  final VoidCallback? onSubmitted;

  /// Whether the field says what is wrong with the address as it is typed.
  /// Off until the first submit, so an empty field is not an error yet.
  final bool validateAsTyped;
  final bool enabled;

  const AddRemoteDeviceForm({
    super.key,
    required this.controller,
    this.formKey,
    this.onSubmitted,
    this.validateAsTyped = false,
    this.enabled = true,
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
              controller: controller,
              enabled: enabled,
              placeholder: '192.168.1.20',
              autofocus: true,
              autovalidateMode: validateAsTyped
                  ? AutovalidateMode.onUserInteraction
                  : AutovalidateMode.disabled,
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
