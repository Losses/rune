import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_manager.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/get_non_replace_operate_mode.dart';
import '../../utils/api/set_adaptive_switching_enabled.dart';
import '../../widgets/settings/settings_box_scrobble_login.dart';
import '../../widgets/settings/settings_box_toggle.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_block_title.dart';
import '../../widgets/settings/settings_box_combo_box.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../widgets/playback_controller/utils/playback_mode.dart';
import '../../widgets/playback_controller/playback_mode_button.dart';

const disabledPlaybackModesKey = 'disabled_playback_modes';
const middleClickActionKey = 'middle_click_action';
const adaptiveSwitchingKey = 'adaptive_switching';

class SettingsPlayback extends StatefulWidget {
  const SettingsPlayback({super.key});

  @override
  State<SettingsPlayback> createState() => _SettingsPlaybackState();
}

class _SettingsPlaybackState extends State<SettingsPlayback> {
  List<PlaybackMode> disabledModes = [];
  String queueMode = "AddToEnd";
  String middleClickAction = "StartPlaying";
  bool adaptiveSwitching = false;

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

    // Load adaptive switching setting
    bool? storedAdaptiveSwitching =
        await SettingsManager().getValue<bool>(adaptiveSwitchingKey);
    if (storedAdaptiveSwitching != null) {
      setState(() {
        adaptiveSwitching = storedAdaptiveSwitching;
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

  Future<void> _updateAdaptiveSwitching(bool newSetting) async {
    setState(() {
      adaptiveSwitching = newSetting;
    });
    await SettingsManager().setValue(adaptiveSwitchingKey, newSetting);
    setAdaptiveSwitchingEnabled();
  }

  @override
  Widget build(BuildContext context) {
    final s = S.of(context);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SingleChildScrollView(
          padding: getScrollContainerPadding(context),
          child: SettingsPagePadding(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SettingsBoxComboBox(
                  title: s.addToQueue,
                  subtitle: s.addToQueueSubtitle,
                  value: queueMode,
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "PlayNext",
                      title: s.playNext,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "AddToEnd",
                      title: s.addToEnd,
                    ),
                  ],
                  onChanged: (newValue) {
                    if (newValue != null) {
                      _updateQueueSetting(newValue);
                    }
                  },
                ),
                SettingsBoxComboBox(
                  title: s.middleClickAction,
                  subtitle: s.middleClickActionSubtitle,
                  value: middleClickAction,
                  items: [
                    SettingsBoxComboBoxItem(
                      value: "StartPlaying",
                      title: s.startPlaying,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "AddToQueue",
                      title: s.addToQueue,
                    ),
                    SettingsBoxComboBoxItem(
                      value: "StartRoaming",
                      title: s.startRoaming,
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
                ),
                SettingsBoxToggle(
                  title: s.adaptiveSwitching,
                  subtitle: s.adaptiveSwitchingSubtitle,
                  value: adaptiveSwitching,
                  onChanged: _updateAdaptiveSwitching,
                ),
                Padding(
                  padding:
                      EdgeInsets.only(top: 8, bottom: 2, left: 6, right: 6),
                  child: Text(s.onlineServices),
                ),
                SettingsBoxScrobbleLogin(
                  title: "Last.fm",
                  subtitle: s.lastFmSubtitle,
                  serviceName: 'LastFm',
                ),
                SettingsBoxScrobbleLogin(
                  title: "Libre.fm",
                  subtitle: s.libreFmSubtitle,
                  serviceName: 'LibreFm',
                ),
                SettingsBoxScrobbleLogin(
                  title: "ListenBrainz",
                  subtitle: s.listenBrainzSubtitle,
                  serviceName: 'ListenBrainz',
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
