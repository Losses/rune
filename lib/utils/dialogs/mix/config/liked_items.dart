import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../../../utils/l10n.dart';

import '../widgets/select_input_section.dart';

List<SelectItem> likedItems(BuildContext context) => [
      SelectItem(
          value: "true",
          title: S.of(context).likedOnly,
          icon: Symbols.heart_check),
      SelectItem(
          value: "false",
          title: S.of(context).allTracks,
          icon: Symbols.done_all),
    ];
