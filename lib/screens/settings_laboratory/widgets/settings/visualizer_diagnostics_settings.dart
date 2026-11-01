import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../../utils/visualizer_supported.dart';
import '../../../../providers/visualizer_scenes.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/visualizer_diagnostics.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class VisualizerDiagnosticsSettings extends StatelessWidget {
  const VisualizerDiagnosticsSettings({super.key});

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) return const SizedBox.shrink();

    final provider = context.watch<VisualizerScenesProvider>();

    final items = visualizerDiagnosticsConfig(context)
        .map((e) => SettingsComboBoxItem(
              value: e.value,
              icon: e.icon,
              text: e.title,
            ))
        .toList();

    return SettingsCard(
      title: "Visualizer Diagnostics",
      description:
          "Also write which GPU the visualizer runs on and how its frames reach the screen to the Log page. Its errors are always there.",
      content: SettingsComboBox<String>(
        value: provider.log ? 'true' : 'false',
        items: items,
        onChanged: (value) {
          if (value == null) return;
          provider.setLog(value == 'true');
        },
      ),
    );
  }
}
