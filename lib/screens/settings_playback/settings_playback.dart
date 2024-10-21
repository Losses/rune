import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../screens/settings_playback/widgets/settings_block.dart';
import '../../screens/settings_playback/widgets/settings_block_title.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/playback_controller/utils/playback_mode.dart';
import '../../widgets/playback_controller/playback_mode_button.dart';

class SettingsPlayback extends StatefulWidget {
  const SettingsPlayback({super.key});

  @override
  State<SettingsPlayback> createState() => _SettingsPlaybackState();
}

class _SettingsPlaybackState extends State<SettingsPlayback> {
  List<PlaybackMode> disabledModes = [];
  String queueSetting = "AddToEnd";

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    // Load disabled playback modes
    List<int>? storedDisabledModes =
        await SettingsManager().getValue<List<int>>('disabledPlaybackModes');
    if (storedDisabledModes != null) {
      setState(() {
        disabledModes = storedDisabledModes
            .map((index) => PlaybackMode.values[index])
            .toList();
      });
    }

    // Load queue setting
    String? storedQueueSetting =
        await SettingsManager().getValue<String>('queueSetting');
    if (storedQueueSetting != null) {
      setState(() {
        queueSetting = storedQueueSetting;
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
    await SettingsManager().setValue('disabledPlaybackModes', modeIndexes);
  }

  Future<void> _updateQueueSetting(String newSetting) async {
    setState(() {
      queueSetting = newSetting;
    });
    await SettingsManager().setValue('queueSetting', newSetting);
  }

  @override
  Widget build(BuildContext context) {
    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: Column(
            children: [
              SettingsBlock(
                title: "Add to Queue",
                subtitle: "How new items to be added to the playback queue.",
                child: ComboBox<String>(
                  value: queueSetting,
                  items: const [
                    ComboBoxItem(
                      value: "PlayNext",
                      child: Text("Play Next"),
                    ),
                    ComboBoxItem(
                      value: "AddToEnd",
                      child: Text("Add to End"),
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateQueueSetting(newValue);
                    }
                  },
                ),
              ),
              Padding(
                padding: const EdgeInsets.all(4),
                child: Expander(
                  header: const Padding(
                    padding: EdgeInsets.symmetric(vertical: 11),
                    child: SettingsBlockTitle(
                      title: "Playback Mode",
                      subtitle:
                          "Preferred playback mode about how your music plays.",
                    ),
                  ),
                  content: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: PlaybackMode.values.map((mode) {
                      return Padding(
                        padding: const EdgeInsets.symmetric(vertical: 4),
                        child: Checkbox(
                          content: Text(modeToLabel(mode)),
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
              )
            ],
          ),
        ),
      ),
    );
  }
}
