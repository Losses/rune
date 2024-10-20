import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/providers/status.dart';

import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';

class ControllerButtons extends StatefulWidget {
  const ControllerButtons({super.key});

  @override
  State<ControllerButtons> createState() => _ControllerButtonsState();
}

class _ControllerButtonsState extends State<ControllerButtons> {
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
    final visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

    final coverArtWallLayout = Provider.of<ResponsiveProvider>(context)
            .smallerOrEqualTo(DeviceType.phone) &&
        GoRouterState.of(context).fullPath == '/cover_wall';

    final miniEntries = [controllerItems[1], controllerItems[2]];

    return Selector<PlaybackStatusProvider, bool>(
      selector: (context, value) => value.notReady,
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
              entry.controllerButtonBuilder(context),
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
                            items: [
                              for (final entry in hiddenEntries)
                                entry.flyoutEntryBuilder(context),
                            ],
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
