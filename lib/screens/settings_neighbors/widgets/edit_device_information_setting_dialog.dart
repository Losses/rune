import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/fingerprint/fingerprint_figure.dart';
import '../../../providers/broadcast.dart';

class EditDeviceInformationSettingDialog extends StatefulWidget {
  final void Function(void) $close;

  const EditDeviceInformationSettingDialog({
    super.key,
    required this.$close,
  });

  @override
  EditDeviceInformationSettingDialogState createState() =>
      EditDeviceInformationSettingDialogState();
}

class EditDeviceInformationSettingDialogState
    extends State<EditDeviceInformationSettingDialog> {
  final TextEditingController deviceNameController = TextEditingController();

  @override
  void initState() {
    final broadcast = Provider.of<BroadcastProvider>(context, listen: false);
    deviceNameController.value =
        TextEditingValue(text: broadcast.deviceAlias ?? "");
    super.initState();
  }

  @override
  void dispose() {
    deviceNameController.dispose();
    super.dispose();
  }

  void _updateDeviceName() {
    final broadcast = Provider.of<BroadcastProvider>(context, listen: false);
    broadcast.updateDeviceAlias(deviceNameController.value.text);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);
    final broadcast = Provider.of<BroadcastProvider>(context);
    final fingerprint = broadcast.fingerprint;

    return NoShortcuts(
      ContentDialog(
        title: Column(
          children: [
            SizedBox(height: 8),
            Text(s.editDeviceInformation),
          ],
        ),
        content: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            const SizedBox(height: 16),
            InfoLabel(
              label: s.deviceName,
              child: TextBox(controller: deviceNameController),
            ),
            const SizedBox(height: 16),
            InfoLabel(
              label: s.deviceFingerprint,
              child: FingerprintFigure(fingerprint: fingerprint),
            ),
          ],
        ),
        actions: [
          FilledButton(
            onPressed: deviceNameController.value.text.isEmpty
                ? null
                : _updateDeviceName,
            child: Text(s.edit),
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
