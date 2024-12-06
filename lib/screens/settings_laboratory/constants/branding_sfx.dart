import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> bandingSfxConfig(BuildContext context) => [
      SelectItem(
        value: 'fantasy',
        title: 'Fantasy',
        icon: Symbols.spa,
      ),
      SelectItem(
        value: 'tech',
        title: 'Technology',
        icon: Symbols.neurology,
      ),
      SelectItem(
        value: 'mute',
        title: 'Disable',
        icon: Symbols.block,
      ),
    ];
