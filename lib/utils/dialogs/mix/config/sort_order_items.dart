import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';
import '../../../../generated/l10n.dart';

List<SelectItem> sortOrderItems(BuildContext context) => [
  SelectItem(value: "true", title: S.of(context).ascending, icon: Symbols.arrow_upward),
  SelectItem(value: "false", title: S.of(context).descending, icon: Symbols.arrow_downward),
];
