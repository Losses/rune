import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../providers/playback_controller.dart';
import '../bindings/bindings.dart';
import '../widgets/capability_gate.dart';

Future<Map<String, MenuFlyoutItem>> fetchFlyoutItems(
  BuildContext context,
) async {
  final entries = Provider.of<PlaybackControllerProvider>(
    context,
    listen: false,
  ).entries;

  final Map<String, MenuFlyoutItem> itemMap = {};

  for (var entry in entries) {
    if (!context.mounted) {
      break;
    }

    final capability = entry.capability(context, false);
    if (capability is RequestCapabilityHidden) continue;

    var item = await entry.flyoutEntryBuilder(context);
    if (!context.mounted) break;
    if (capability case RequestCapabilityDisabled(:final reason)) {
      final reasonText = capabilityReasonText(context, reason);
      item = MenuFlyoutItem(
        key: item.key,
        leading: item.leading,
        text: item.text,
        trailing: Tooltip(
          message: reasonText,
          child: const Icon(FluentIcons.info),
        ),
        selected: item.selected,
        closeAfterClick: item.closeAfterClick,
        onPressed: null,
      );
    }
    itemMap[entry.id] = item;
  }

  if (!context.mounted) {
    return itemMap;
  }

  return itemMap;
}
