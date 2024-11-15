import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';
import '../../../../generated/l10n.dart';

List<SelectItem> sortSelectItems(BuildContext context) => [
      SelectItem(
          value: "default",
          title: S.of(context).defaultMode,
          icon: Symbols.stream),
      SelectItem(
          value: "last_modified",
          title: S.of(context).lastModified,
          icon: Symbols.refresh),
      SelectItem(
          value: "duration",
          title: S.of(context).duration,
          icon: Symbols.access_time_filled),
      SelectItem(
          value: "playedthrough",
          title: S.of(context).timesPlayedThrough,
          icon: Symbols.line_end_circle),
      SelectItem(
          value: "skipped",
          title: S.of(context).timesSkipped,
          icon: Symbols.step_over),
    ];
