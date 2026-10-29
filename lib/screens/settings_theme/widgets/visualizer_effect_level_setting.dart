import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../utils/visualizer_effect_level.dart';
import '../../../utils/visualizer_supported.dart';
import '../../../widgets/settings/settings_box_combo_box.dart';

/// How often the visualizer plays effects at the music's moments. The choice is
/// saved here and sent to Rust, so a running visualizer follows at once; one
/// that is not open gets it when it starts.
class VisualizerEffectLevelSetting extends StatefulWidget {
  const VisualizerEffectLevelSetting({super.key});

  @override
  State<VisualizerEffectLevelSetting> createState() =>
      _VisualizerEffectLevelSettingState();
}

class _VisualizerEffectLevelSettingState
    extends State<VisualizerEffectLevelSetting> {
  VisualizerEffectLevel _level = VisualizerEffectLevel.normal;

  @override
  void initState() {
    super.initState();
    if (isVisualizerSupported) _load();
  }

  Future<void> _load() async {
    final level = await loadVisualizerEffectLevel();
    if (!mounted) return;
    setState(() => _level = level);
  }

  @override
  Widget build(BuildContext context) {
    if (!isVisualizerSupported) {
      return const SizedBox.shrink();
    }

    final s = S.of(context);

    return SettingsBoxComboBox<VisualizerEffectLevel>(
      title: s.visualizerEffects,
      subtitle: s.visualizerEffectsSubtitle,
      value: _level,
      items: [
        SettingsBoxComboBoxItem(
          value: VisualizerEffectLevel.normal,
          title: s.visualizerEffectsDefault,
        ),
        SettingsBoxComboBoxItem(
          value: VisualizerEffectLevel.more,
          title: s.visualizerEffectsMore,
        ),
      ],
      onChanged: (level) {
        if (level == null || level == _level) return;
        setState(() => _level = level);
        saveVisualizerEffectLevel(level);
      },
    );
  }
}
