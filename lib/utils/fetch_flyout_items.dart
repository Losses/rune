import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/playback_controller.dart';

Future<Map<String, MenuFlyoutItem>> fetchFlyoutItems(
    BuildContext context) async {
  final entries =
      Provider.of<PlaybackControllerProvider>(context, listen: false).entries;

  final Map<String, MenuFlyoutItem> itemMap = {};

  for (var entry in entries) {
    if (!context.mounted) {
      break;
    }

    final item = await entry.flyoutEntryBuilder(context);
    itemMap[entry.id] = item;
  }

  if (!context.mounted) {
    return itemMap;
  }

  return itemMap;
}
