import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/fetch_flyout_items.dart';
import '../../utils/is_cover_art_wall_layout.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/unavailable_menu_entry.dart';
import '../../widgets/ax_reveal/ax_reveal.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../providers/status.dart';
import '../../providers/router_path.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';
import '../rune_clickable.dart';

class ControllerButtons extends StatefulWidget {
  const ControllerButtons({super.key});

  @override
  State<ControllerButtons> createState() => _ControllerButtonsState();
}

class _ControllerButtonsState extends State<ControllerButtons> {
  Map<String, MenuFlyoutItem> flyoutItems = {};
  Locale? currentLocale;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _checkLocaleAndFetchItems();
  }

  void _checkLocaleAndFetchItems() {
    final newLocale = Localizations.localeOf(context);
    if (currentLocale != newLocale) {
      _fetchFlyoutItems(newLocale);
    }
  }

  Future<void> _fetchFlyoutItems(Locale locale) async {
    currentLocale = locale;
    final Map<String, MenuFlyoutItem> itemMap = await fetchFlyoutItems(context);

    if (!context.mounted) {
      return;
    }

    setState(() {
      flyoutItems = itemMap;
    });
  }

  final _menuController = RouterAwareFlyoutController();

  @override
  void dispose() {
    super.dispose();
    _menuController.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final miniLayout = Provider.of<ResponsiveProvider>(context)
        .smallerOrEqualTo(DeviceType.mobile);

    final controllerProvider = Provider.of<PlaybackControllerProvider>(context);

    final path = Provider.of<RouterPathProvider>(context).path;

    final coverArtWallLayout = Provider.of<ResponsiveProvider>(context)
            .smallerOrEqualTo(DeviceType.phone) &&
        isCoverArtWallLayout(path);

    final miniEntries = [controllerItems[1], controllerItems[2]];

    Provider.of<PlaybackStatusProvider>(context);

    final entries = controllerProvider.entries;
    final hiddenIndex = entries.indexWhere((entry) => entry.id == 'hidden');
    final List<ControllerEntry> visibleEntries =
        hiddenIndex != -1 ? entries.sublist(0, hiddenIndex) : entries;
    final List<ControllerEntry> hiddenEntries =
        hiddenIndex != -1 ? entries.sublist(hiddenIndex + 1) : [];

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
            child: AxReveal0(
              child: entry.controllerButtonBuilder(context, null),
            ),
          ),
        if (hiddenEntries.isNotEmpty)
          FlyoutTarget(
            controller: _menuController.controller,
            child: AxReveal0(
              child: RuneClickable(
                child: const Icon(Symbols.more_vert),
                onPressed: () async {
                  await _fetchFlyoutItems(Localizations.localeOf(context));

                  _menuController.showFlyout(
                    builder: (context) {
                      return Container(
                        constraints: const BoxConstraints(maxWidth: 200),
                        child: MenuFlyout(
                          items: ((miniLayout && !coverArtWallLayout)
                                  ? entries
                                  : hiddenEntries)
                              .map(
                                (x) =>
                                    flyoutItems[x.id] ??
                                    unavailableMenuEntry(context),
                              )
                              .toList(),
                        ),
                      );
                    },
                  );
                },
              ),
            ),
          ),
        const SizedBox(width: 8),
      ],
    );
  }
}
