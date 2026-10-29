import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';
import '../../../providers/cover_wall_display.dart';

class CoverWallDisplaySetting extends StatelessWidget {
  const CoverWallDisplaySetting({super.key});

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);
    final provider = context.watch<CoverWallDisplayProvider>();

    return SettingsBoxComboBox<CoverWallDisplay>(
      title: s.coverWallDisplay,
      subtitle: s.coverWallDisplaySubtitle,
      value: provider.display,
      items: [
        SettingsBoxComboBoxItem(
          value: CoverWallDisplay.covers,
          title: s.coverWallDisplayCovers,
        ),
        SettingsBoxComboBoxItem(
          value: CoverWallDisplay.visualizer,
          title: s.coverWallDisplayVisualizer,
        ),
      ],
      onChanged: (display) {
        if (display != null) provider.setDisplay(display);
      },
    );
  }
}
