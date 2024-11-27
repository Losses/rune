import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> randomCoverWallCountConfig(BuildContext context) => [
      SelectItem(
        value: '40',
        title: 'Up to 40 cover arts',
        icon: Symbols.square,
      ),
      SelectItem(
        value: '80',
        title: 'Up to 80 cover arts',
        icon: Symbols.background_dot_large,
      ),
      SelectItem(
        value: '160',
        title: 'Up to 160 cover arts',
        icon: Symbols.background_dot_small,
      ),
    ];
