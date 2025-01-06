import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_page_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';

import 'widgets/performance_level_setting.dart';

class SettingsAnalysis extends StatefulWidget {
  const SettingsAnalysis({super.key});

  @override
  State<SettingsAnalysis> createState() => _SettingsAnalysisState();
}

class _SettingsAnalysisState extends State<SettingsAnalysis> {
  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              children: const [
                // The setting has been hidden due to significant stability issues.
                // ComputingDeviceSetting(),
                PerformanceLevelSetting(),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
