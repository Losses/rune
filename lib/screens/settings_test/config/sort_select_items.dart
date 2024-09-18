import 'package:material_symbols_icons/symbols.dart';

import '../../../screens/settings_test/widgets/select_input_section.dart';

final sortSelectItems = [
  SelectItem(value: "default", title: "Default", icon: Symbols.stream),
  SelectItem(
      value: "last_modified", title: "Last Modified", icon: Symbols.refresh),
  SelectItem(
      value: "duration", title: "Duration", icon: Symbols.access_time_filled),
  SelectItem(
      value: "playedthrough",
      title: "Times Played Through",
      icon: Symbols.line_end_circle),
  SelectItem(value: "skipped", title: "Times Skipped", icon: Symbols.step_over),
];