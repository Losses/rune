import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../../utils/l10n.dart';
import '../../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> backgroundItem(BuildContext context) => [
      SelectItem(
        value: 'dark',
        title: S.of(context).dark,
        icon: Symbols.dark_mode,
      ),
      SelectItem(
        value: 'light',
        title: S.of(context).light,
        icon: Symbols.light_mode,
      ),
    ];
