import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/no_shortcuts.dart';
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
    final theme = FluentTheme.of(context);
    final broadcast = Provider.of<BroadcastProvider>(context);
    final fingerprint = broadcast.fingerprint;

    return NoShortcuts(
      ContentDialog(
        title: Text(s.editDeviceInformation),
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
              child: LayoutBuilder(
                builder: (context, constraints) {
                  return GridView.count(
                    crossAxisCount: 4,
                    childAspectRatio: 2,
                    mainAxisSpacing: 4,
                    crossAxisSpacing: 4,
                    physics: const NeverScrollableScrollPhysics(),
                    shrinkWrap: true,
                    children: List.generate(20, (index) {
                      final startIndex = index * 2;
                      final text = fingerprint == null ||
                              startIndex >= fingerprint.length
                          ? ''
                          : '${fingerprint[startIndex]}${startIndex + 1 < fingerprint.length ? fingerprint[startIndex + 1] : ''}';
                      return TextBox(
                        readOnly: true,
                        placeholder: text,
                        placeholderStyle: TextStyle(
                          color: theme.resources.textFillColorPrimary,
                        ),
                        style: const TextStyle(
                          fontFamily: 'NotoRunic',
                          fontSize: 20,
                          letterSpacing: 4,
                        ),
                        textAlign: TextAlign.center,
                      );
                    }),
                  );
                },
              ),
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
