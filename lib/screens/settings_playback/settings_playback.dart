import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/get_non_replace_operate_mode.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_block_title.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/playback_controller/utils/playback_mode.dart';
import '../../widgets/playback_controller/playback_mode_button.dart';

const disabledPlaybackModesKey = 'disabled_playback_modes';
const middleClickActionKey = 'middle_click_action';

class SettingsPlayback extends StatefulWidget {
  const SettingsPlayback({super.key});

  @override
  State<SettingsPlayback> createState() => _SettingsPlaybackState();
}

class _SettingsPlaybackState extends State<SettingsPlayback> {
  List<PlaybackMode> disabledModes = [];
  String queueMode = "AddToEnd";
  String middleClickAction = "StartPlaying";

  @override
  void initState() {
    super.initState();
    _loadSettings();
  }

  Future<void> _loadSettings() async {
    // Load disabled playback modes
    List<dynamic>? storedDisabledModes = await SettingsManager()
        .getValue<List<dynamic>>(disabledPlaybackModesKey);
    if (storedDisabledModes != null) {
      setState(() {
        disabledModes = storedDisabledModes
            .map((index) => PlaybackMode.values[index as int])
            .toList();
      });
    }

    // Load queue setting
    String? storedQueueSetting =
        await SettingsManager().getValue<String>(nonReplaceOperateModeKey);
    if (storedQueueSetting != null) {
      setState(() {
        queueMode = storedQueueSetting;
      });
    }

    // Load middle-click action setting
    String? storedMiddleClickAction =
        await SettingsManager().getValue<String>(middleClickActionKey);
    if (storedMiddleClickAction != null) {
      setState(() {
        middleClickAction = storedMiddleClickAction;
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
    await SettingsManager().setValue(disabledPlaybackModesKey, modeIndexes);
  }

  Future<void> _updateQueueSetting(String newSetting) async {
    setState(() {
      queueMode = newSetting;
    });
    await SettingsManager().setValue(nonReplaceOperateModeKey, newSetting);
  }

  Future<void> _updateMiddleClickAction(String newAction) async {
    setState(() {
      middleClickAction = newAction;
    });
    await SettingsManager().setValue(middleClickActionKey, newAction);
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
                  title: "Add to Queue",
                  subtitle: "How new items to be added to the playback queue.",
                  value: queueMode,
                  items: const [
                    SettingsBoxComboBoxItem(
                      value: "PlayNext",
                      title: "Play Next",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "AddToEnd",
                      title: "Add to End",
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateQueueSetting(newValue);
                    }
                  },
                ),
                SettingsBoxComboBox(
                  title: "Middle Click Action",
                  subtitle:
                      "Action to perform when middle-clicking a track or collection.",
                  value: middleClickAction,
                  items: const [
                    SettingsBoxComboBoxItem(
                      value: "StartPlaying",
                      title: "Start Playing",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "AddToQueue",
                      title: "Add to Queue",
                    ),
                    SettingsBoxComboBoxItem(
                      value: "StartRoaming",
                      title: "Start Roaming",
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateMiddleClickAction(newValue);
                    }
                  },
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
      ),
    );
  }
}
