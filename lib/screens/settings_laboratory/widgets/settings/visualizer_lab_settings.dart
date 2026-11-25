import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../../utils/visualizer_supported.dart';
import '../../../../providers/visualizer_lab.dart';
import '../../../../providers/visualizer_scenes.dart';

import '../../utils/settings_combo_box_item.dart';
import '../../constants/visualizer_lab.dart';

import '../settings_card.dart';
import '../settings_combo_box.dart';

class VisualizerLabSettings extends StatelessWidget {
  const VisualizerLabSettings({super.key});

  @override
  Widget build(BuildContext context) {
    // Without a GPU for the scenes there is no visualizer to look into.
    final gpu = context.select<VisualizerScenesProvider, bool?>((p) => p.gpu);
    if (!isVisualizerSupported || gpu != true) return const SizedBox.shrink();

    final provider = context.watch<VisualizerLabProvider>();

    final items = visualizerLabConfig(context)
        .map(
          (e) =>
              SettingsComboBoxItem(value: e.value, icon: e.icon, text: e.title),
        )
        .toList();

    return SettingsCard(
      title: "Visualizer Lab",
      description:
          "Add an item to the playback controller's menu that opens the lab over the visualizer page.",
      content: SettingsComboBox<String>(
        value: provider.enabled ? 'true' : 'false',
        items: items,
        onChanged: (value) {
          if (value == null) return;
          provider.setEnabled(value == 'true');
        },
      ),
    );
  }
}
