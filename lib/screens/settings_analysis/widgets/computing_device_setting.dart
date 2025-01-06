import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class ComputingDeviceSetting extends StatefulWidget {
  const ComputingDeviceSetting({super.key});

  @override
  ComputingDeviceSettingState createState() => ComputingDeviceSettingState();
}

class ComputingDeviceSettingState extends State<ComputingDeviceSetting> {
  String computingDevice = "cpu";

  @override
  void initState() {
    super.initState();
    _loadComputingDevice();
  }

  Future<void> _loadComputingDevice() async {
    final storedComputingDevice =
        await $settingsManager.getValue<String>(kAnalysisComputingDeviceKey);
    setState(() {
      computingDevice = storedComputingDevice ?? "cpu";
    });
  }

  Future<void> _updateComputingDevice(String newDevice) async {
    setState(() {
      computingDevice = newDevice;
    });
    await $settingsManager.setValue(kAnalysisComputingDeviceKey, newDevice);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return SettingsBoxComboBox(
      title: s.computingDevice,
      subtitle: s.computingDeviceSubtitle,
      value: computingDevice,
      items: [
        SettingsBoxComboBoxItem(value: "gpu", title: s.gpu),
        SettingsBoxComboBoxItem(value: "cpu", title: s.cpu),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateComputingDevice(newValue);
        }
      },
    );
  }
}
