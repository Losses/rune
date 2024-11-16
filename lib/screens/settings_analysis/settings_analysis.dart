import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../utils/l10n.dart';

const analysisComputingDeviceKey = 'analysis_mode';
const analysisPerformanceLevelKey = 'analysis_performance';

class SettingsAnalysis extends StatefulWidget {
  const SettingsAnalysis({super.key});

  @override
  State<SettingsAnalysis> createState() => _SettingsAnalysisState();
}

class _SettingsAnalysisState extends State<SettingsAnalysis> {
  String computingDevice = "cpu";
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

  Future<void> _updatePerformanceLevel(String newSetting) async {
    setState(() {
      performanceLevel = newSetting;
    });
    await SettingsManager().setValue(analysisPerformanceLevelKey, newSetting);
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              children: [
                SettingsBoxComboBox(
                  title: S.of(context).computingDevice,
                  subtitle: S.of(context).computingDeviceSubtitle,
                  value: computingDevice,
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "gpu",
                      title: S.of(context).gpu,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "cpu",
                      title: S.of(context).cpu,
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateAnalysisDevice(newValue);
                    }
                  },
                ),
                SettingsBoxComboBox(
                  title: S.of(context).performanceLevel,
                  subtitle: S.of(context).performanceLevelSubtitle,
                  value: performanceLevel,
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "performance",
                      title: S.of(context).performance,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "balance",
                      title: S.of(context).balance,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "battery",
                      title: S.of(context).batterySaving,
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updatePerformanceLevel(newValue);
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
