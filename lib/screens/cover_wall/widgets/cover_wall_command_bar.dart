import 'package:fluent_ui/fluent_ui.dart';

import '../utils/primary_command_bar_item.dart';
import '../utils/overflow_command_bar_item.dart';

import '../../../utils/unavailable_menu_entry.dart';
import '../../../widgets/playback_controller/constants/controller_items.dart';

class CoverWallCommandBar extends StatelessWidget {
  const CoverWallCommandBar({
    super.key,
    required this.flyoutItems,
    required this.entries,
    required this.shadows,
  });

  final (List<ControllerEntry>, List<ControllerEntry>) entries;
  final Map<String, MenuFlyoutItem> flyoutItems;
  final List<Shadow> shadows;

  @override
  Widget build(BuildContext context) {
    return CommandBar(
      isCompact: true,
      overflowMenuItemBuilder: (context, entry) {
        if (entry is PrimaryCommandBarItem) {
          final item = flyoutItems[entry.entry.id];
          if (item != null) {
            return item;
          }
          return unavailableMenuEntry(context);
        }

        throw "Unacceptable entry type";
      },
      overflowItemBuilder: (onPressed) {
        return OverflowCommandBarItem(
          key: const ValueKey("Overflow Item"),
          onPressed: onPressed,
        );
      },
      primaryItems: entries.$1
          .map(
            (x) => PrimaryCommandBarItem(
              key: ValueKey(x.id),
              entry: x,
              shadows: shadows,
            ),
          )
          .toList(),
      secondaryItems: entries.$2
          .map(
            (x) => PrimaryCommandBarItem(
              key: ValueKey(x.id),
              entry: x,
              shadows: shadows,
            ),
          )
          .toList(),
    );
  }
}
