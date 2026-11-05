import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../../utils/dialogs/mix/widgets/select_input_section.dart';

List<SelectItem> visualizerLabConfig(BuildContext context) => [
  SelectItem(value: 'true', title: 'Enable', icon: Symbols.science),
  SelectItem(value: 'false', title: 'Disable', icon: Symbols.block),
];
