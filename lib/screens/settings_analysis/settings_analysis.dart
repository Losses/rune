import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

const analysisComputingDeviceKey = 'analysis_mode';
const analysisPerformanceLevelKey = 'analysis_performance';

class SettingsAnalysis extends StatefulWidget {
  const SettingsAnalysis({super.key});

  @override
  State<SettingsAnalysis> createState() => _SettingsAnalysisState();
}

class _SettingsAnalysisState extends State<SettingsAnalysis> {
  String computingDevice = "gpu";
  String performanceLevel = "performance";

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    String? storedComputingDeviceKey =
        await SettingsManager().getValue<String>(analysisComputingDeviceKey);
    String? storedPerformanceLevel =
        await SettingsManager().getValue<String>(analysisPerformanceLevelKey);
    setState(() {
      if (storedComputingDeviceKey != null) {
        computingDevice = storedComputingDeviceKey;
      }
      if (storedPerformanceLevel != null) {
        performanceLevel = storedPerformanceLevel;
      }
    });
  }

  Future<void> _updateAnalysisDevice(String newSetting) async {
    setState(() {
      computingDevice = newSetting;
    });
    await SettingsManager().setValue(analysisComputingDeviceKey, newSetting);
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          child: SettingsPagePadding(
            child: Column(
              children: [
                SettingsBoxComboBox(
                  title: "Computing Device",
                  subtitle:
                      "Select GPU or CPU for faster or more efficient processing.",
                  value: computingDevice,
                  items: const [
                    SettingsBoxComboBoxItem(
                      value: "gpu",
                      title: "GPU",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "cpu",
                      title: "CPU",
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateAnalysisDevice(newValue);
                    }
                  },
                ),
                SettingsBoxComboBox(
                  title: "Performance Level",
                  subtitle: "Choose how many tasks to run simultaneously.",
                  value: performanceLevel,
                  items: const [
                    SettingsBoxComboBoxItem(
                      value: "performance",
                      title: "Performance",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "balance",
                      title: "Balance",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "battery",
                      title: "Battery saving",
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateAnalysisDevice(newValue);
                    }
                  },
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
