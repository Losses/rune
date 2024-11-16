import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../widgets/select_input_section.dart';

import '../../../../utils/l10n.dart';

List<SelectItem> recommendSelectItems(BuildContext context) => [
      SelectItem(
          value: "",
          title: S.of(context).noRecommendation,
          icon: Symbols.circles_ext),
      SelectItem(
          value: "-1",
          title: S.of(context).basedOnAll,
          icon: Symbols.blur_circular),
      SelectItem(
          value: "0", title: S.of(context).group1, icon: Symbols.counter_1),
      SelectItem(
          value: "1", title: S.of(context).group2, icon: Symbols.counter_2),
      SelectItem(
          value: "2", title: S.of(context).group3, icon: Symbols.counter_3),
      SelectItem(
          value: "3", title: S.of(context).group4, icon: Symbols.counter_4),
      SelectItem(
          value: "4", title: S.of(context).group5, icon: Symbols.counter_5),
      SelectItem(
          value: "5", title: S.of(context).group6, icon: Symbols.counter_6),
      SelectItem(
          value: "6", title: S.of(context).group7, icon: Symbols.counter_7),
      SelectItem(
          value: "7", title: S.of(context).group8, icon: Symbols.counter_8),
      SelectItem(
          value: "8", title: S.of(context).group9, icon: Symbols.counter_9),
    ];
