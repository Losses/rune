import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/settings_manager.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../constants/configurations.dart';

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
        await SettingsManager().getValue<String>(kAnalysisComputingDeviceKey);
    setState(() {
      computingDevice = storedComputingDevice ?? "cpu";
    });
  }

  Future<void> _updateComputingDevice(String newDevice) async {
    setState(() {
      computingDevice = newDevice;
    });
    await SettingsManager().setValue(kAnalysisComputingDeviceKey, newDevice);
  }

  @override
  Widget build(BuildContext context) {
    return SettingsBoxComboBox(
      title: S.of(context).computingDevice,
      subtitle: S.of(context).computingDeviceSubtitle,
      value: computingDevice,
      items: [
        SettingsBoxComboBoxItem(value: "gpu", title: S.of(context).gpu),
        SettingsBoxComboBoxItem(value: "cpu", title: S.of(context).cpu),
      ],
      onChanged: (newValue) {
        if (newValue != null) {
          _updateComputingDevice(newValue);
        }
      },
    );
  }
}
