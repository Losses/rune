import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> sizeItems(BuildContext context) => [
      SelectItem(
        value: '16 9',
        title: '¹⁶⁄₉',
        icon: Symbols.crop_16_9,
      ),
      SelectItem(
        value: '3 2',
        title: '³⁄₂',
        icon: Symbols.crop_3_2,
      ),
      SelectItem(
        value: '7 5',
        title: '⁷⁄₅',
        icon: Symbols.crop_7_5,
      ),
      SelectItem(
        value: '5 4',
        title: '⁵⁄₄',
        icon: Symbols.crop_5_4,
      ),
      SelectItem(
        value: '1 1',
        title: '¹⁄₁',
        icon: Symbols.crop_square,
      ),
      SelectItem(
        value: '9 16',
        title: '⁹⁄₁₆',
        icon: Symbols.crop_9_16,
      ),
    ];
