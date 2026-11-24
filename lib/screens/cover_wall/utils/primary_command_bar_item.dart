import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/playback_controller/constants/controller_items.dart';
import '../../../widgets/capability_gate.dart';

class PrimaryCommandBarItem extends CommandBarItem {
  PrimaryCommandBarItem({
    required super.key,
    required this.entry,
    required this.shadows,
  });

  final List<Shadow>? shadows;
  final ControllerEntry entry;

  @override
  Widget build(BuildContext context, CommandBarItemDisplayMode displayMode) {
    return Tooltip(
      message: entry.tooltipBuilder(context),
      child: CapabilityGate(
        capability: entry.capability(context, true),
        child: entry.controllerButtonBuilder(context, shadows),
      ),
    );
  }
}
