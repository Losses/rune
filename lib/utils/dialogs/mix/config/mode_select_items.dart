import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';

import '../../../../widgets/playback_controller/utils/playback_mode.dart';
import '../../../../widgets/playback_controller/playback_mode_button.dart';
import '../../../../utils/l10n.dart';

List<SelectItem> modeSelectItems(BuildContext context) => [
      SelectItem(
          value: "99",
          title: S.of(context).defaultMode,
          icon: Symbols.change_history),
      ...[PlaybackMode.sequential, PlaybackMode.repeatAll, PlaybackMode.shuffle]
          .map(
        (x) => SelectItem(
          value: modeToInt(x).toString(),
          title: modeToLabel(context, x),
          icon: modeToIcon(x),
        ),
      ),
    ];
