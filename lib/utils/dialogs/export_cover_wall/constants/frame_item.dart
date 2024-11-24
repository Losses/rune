import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../../utils/l10n.dart';
import '../../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> frameItem(BuildContext context) => <SelectItem>[
      SelectItem(
        value: 'enable',
        title: S.of(context).enable,
        icon: Symbols.iframe,
      ),
      SelectItem(
        value: 'disable',
        title: S.of(context).disable,
        icon: Symbols.iframe_off,
      ),
    ];
