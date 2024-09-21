import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';
import '../../../../widgets/playback_controller/utils/playback_mode.dart';
import '../../../../widgets/playback_controller/playback_mode_button.dart';

final modeSelectItems = [
  SelectItem(value: "99", title: "Default", icon: Symbols.change_history),
  ...[PlaybackMode.sequential, PlaybackMode.repeatAll, PlaybackMode.shuffle]
      .map(
    (x) => SelectItem(
      value: modeToInt(x).toString(),
      title: modeToLabel(x),
      icon: modeToIcon(x),
    ),
  ),
];