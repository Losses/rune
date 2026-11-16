import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';

import '../../../../bindings/bindings.dart';
import '../../../../widgets/playback_controller/playback_mode_button.dart';
import '../../../../utils/l10n.dart';

/// The option that keeps the player's playback mode.
const keepModeSelectValue = 'keep';

const _selectableModes = [
  PlaybackMode.sequential,
  PlaybackMode.repeatAll,
  PlaybackMode.shuffle,
];

/// The option of the mode selector that stands for [mode].
String modeSelectValue(PlaybackMode? mode) =>
    mode == null ? keepModeSelectValue : mode.index.toString();

/// The playback mode an option of the mode selector stands for, `null` for keeping the
/// player's mode.
PlaybackMode? playbackModeFromSelectValue(String? value) {
  for (final mode in _selectableModes) {
    if (modeSelectValue(mode) == value) return mode;
  }
  return null;
}

List<SelectItem> modeSelectItems(BuildContext context) => [
  SelectItem(
    value: keepModeSelectValue,
    title: S.of(context).defaultMode,
    icon: Symbols.change_history,
  ),
  ..._selectableModes.map(
    (x) => SelectItem(
      value: modeSelectValue(x),
      title: modeToLabel(context, x),
      icon: modeToIcon(x),
    ),
  ),
];
