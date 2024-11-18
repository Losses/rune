import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/playback_controller/constants/controller_items.dart';

class PrimaryCommandBarItem extends CommandBarItem {
  PrimaryCommandBarItem(
      {required super.key, required this.entry, required this.shadows});

  final List<Shadow>? shadows;
  final ControllerEntry entry;

  @override
  Widget build(BuildContext context, CommandBarItemDisplayMode displayMode) {
    return Tooltip(
      message: entry.tooltipBuilder(context),
      child: entry.controllerButtonBuilder(context, shadows),
    );
  }
}
