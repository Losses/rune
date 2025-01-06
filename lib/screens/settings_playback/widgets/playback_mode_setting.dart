import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../../widgets/playback_controller/utils/playback_mode.dart';
import '../../../widgets/playback_controller/playback_mode_button.dart';
import '../../../widgets/settings/settings_block_title.dart';
import '../../../constants/configurations.dart';
import '../../../constants/settings_manager.dart';

class PlaybackModeSetting extends StatefulWidget {
  const PlaybackModeSetting({super.key});

  @override
  PlaybackModeSettingState createState() => PlaybackModeSettingState();
}

class PlaybackModeSettingState extends State<PlaybackModeSetting> {
  List<PlaybackMode> disabledModes = [];

  @override
  void initState() {
    super.initState();
    _loadDisabledModes();
  }

  Future<void> _loadDisabledModes() async {
    final storedDisabledModes = await $settingsManager
        .getValue<List<dynamic>>(kDisabledPlaybackModesKey);
    if (storedDisabledModes != null) {
      setState(() {
        disabledModes = storedDisabledModes
            .map((index) => PlaybackMode.values[index as int])
            .toList();
      });
    }
  }

  Future<void> _updateDisabledModes(PlaybackMode mode, bool isDisabled) async {
    setState(() {
      if (isDisabled) {
        disabledModes.add(mode);
      } else {
        disabledModes.remove(mode);
      }
    });
    List<int> modeIndexes =
        disabledModes.map((mode) => modeToInt(mode)).toList();
    await $settingsManager.setValue(kDisabledPlaybackModesKey, modeIndexes);
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return Padding(
      padding: const EdgeInsets.all(4),
      child: Expander(
        header: Padding(
          padding: const EdgeInsets.symmetric(vertical: 11),
          child: SettingsBlockTitle(
            title: s.playbackMode,
            subtitle: s.playbackModeSubtitle,
          ),
        ),
        content: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: PlaybackMode.values.map((mode) {
            return Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Checkbox(
                content: Text(modeToLabel(context, mode)),
                checked: !disabledModes.contains(mode),
                onChanged: (isChecked) {
                  if (isChecked != null) {
                    _updateDisabledModes(mode, !isChecked);
                  }
                },
              ),
            );
          }).toList(),
        ),
      ),
    );
  }
}
