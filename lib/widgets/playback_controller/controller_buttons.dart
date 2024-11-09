import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/fetch_flyout_items.dart';
import '../../utils/unavailable_menu_entry.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../messages/playback.pb.dart';
import '../../providers/status.dart';
import '../../providers/router_path.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

class ControllerButtons extends StatefulWidget {
  const ControllerButtons({super.key});

  @override
  State<ControllerButtons> createState() => _ControllerButtonsState();
}

class _ControllerButtonsState extends State<ControllerButtons> {
  Map<String, MenuFlyoutItem> flyoutItems = {};
  bool initialized = false;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    _fetchFlyoutItems();
  }

  Future<void> _fetchFlyoutItems() async {
    if (initialized) return;
    initialized = true;

    final Map<String, MenuFlyoutItem> itemMap = await fetchFlyoutItems(context);

    if (!context.mounted) {
      return;
    }

    setState(() {
      flyoutItems = itemMap;
    });
  }

  final menuController = FlyoutController();

  @override
  void dispose() {
    super.dispose();
    menuController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final miniLayout = Provider.of<ResponsiveProvider>(context)
        .smallerOrEqualTo(DeviceType.mobile);

    final controllerProvider = Provider.of<PlaybackControllerProvider>(context);
    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final List<ControllerEntry> visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final List<ControllerEntry> hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    final path = Provider.of<RouterPathProvider>(context).path;

    final coverArtWallLayout = Provider.of<ResponsiveProvider>(context)
            .smallerOrEqualTo(DeviceType.phone) &&
        path == '/cover_wall';

    final miniEntries = [controllerItems[1], controllerItems[2]];

    return Selector<PlaybackStatusProvider, (bool, PlaybackStatus?)>(
      selector: (context, value) => (value.notReady, value.playbackStatus),
      builder: (context, value, child) {
        return Row(
          mainAxisAlignment: coverArtWallLayout
              ? MainAxisAlignment.spaceAround
              : MainAxisAlignment.end,
          children: [
            if (coverArtWallLayout) const SizedBox(width: 8),
            for (final entry in (miniLayout && !coverArtWallLayout)
                ? miniEntries
                : visibleEntries)
              Tooltip(
                  message: entry.tooltipBuilder(context),
                  child: entry.controllerButtonBuilder(context, null)),
            if (hiddenEntries.isNotEmpty)
              FlyoutTarget(
                controller: menuController,
                child: IconButton(
                  icon: const Icon(Symbols.more_vert),
                  onPressed: () {
                    menuController.showFlyout(
                      builder: (context) {
                        return Container(
                          constraints: const BoxConstraints(maxWidth: 200),
                          child: MenuFlyout(
                            items: hiddenEntries
                                .map(
                                  (x) =>
                                      flyoutItems[x.id] ?? unavailableMenuEntry,
                                )
                                .toList(),
                          ),
                        );
                      },
                    );
                  },
                ),
              ),
            const SizedBox(width: 8),
          ],
        );
      },
    );
  }
}
