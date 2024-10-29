import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/playback_controller/constants/controller_items.dart';

class PrimaryCommandBarItem extends CommandBarItem {
  PrimaryCommandBarItem({required super.key, required this.entry});

  final ControllerEntry entry;

  @override
  Widget build(BuildContext context, CommandBarItemDisplayMode displayMode) {
    return entry.controllerButtonBuilder(context);
  }
}
