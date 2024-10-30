import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

class OverflowCommandBarItem extends CommandBarItem {
  OverflowCommandBarItem({required super.key, required this.onPressed});

  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context, CommandBarItemDisplayMode displayMode) {
    return IconButton(
      icon: const Icon(Symbols.more_vert),
      onPressed: onPressed,
    );
  }
}
