import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/settings_library/widgets/settings_button.dart';
import '../../providers/volume.dart';
import '../../providers/playback_controller.dart';

class SettingsMediaControllerPage extends StatefulWidget {
  const SettingsMediaControllerPage({super.key});

  @override
  State<SettingsMediaControllerPage> createState() =>
      _SettingsMediaControllerPageState();
}

class _SettingsMediaControllerPageState
    extends State<SettingsMediaControllerPage> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    final playbackController = Provider.of<PlaybackControllerProvider>(context);
    Provider.of<VolumeProvider>(context);

    return PageContentFrame(
      child: UnavailablePageOnBand(
        child: SettingsPagePadding(
          child: SingleChildScrollView(
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                    width: double.maxFinite,
                    child: ReorderableColumn(
                      needsLongPressDraggable: false,
                      onReorder: playbackController.reorder,
                      children: playbackController.entries
                          .map(
                            (item) => item.id == 'hidden'
                                ? Padding(
                                    key: const ValueKey("hidden"),
                                    padding: const EdgeInsets.symmetric(
                                      vertical: 12,
                                      horizontal: 8,
                                    ),
                                    child: Row(
                                      children: [
                                        Expanded(
                                          child: Container(
                                            height: 1,
                                            width: double.infinity,
                                            color: theme.inactiveColor
                                                .withAlpha(40),
                                          ),
                                        ),
                                        Padding(
                                          padding: const EdgeInsets.symmetric(
                                            horizontal: 24,
                                          ),
                                          child: Text(
                                            "Action Menu",
                                            style: TextStyle(
                                              color: theme.inactiveColor
                                                  .withAlpha(160),
                                            ),
                                          ),
                                        ),
                                        Expanded(
                                          child: Container(
                                            height: 1,
                                            width: double.infinity,
                                            color: theme.inactiveColor
                                                .withAlpha(40),
                                          ),
                                        ),
                                      ],
                                    ),
                                  )
                                : SettingsButton(
                                    key: ValueKey(item.id),
                                    icon: item.icon(context),
                                    suffixIcon: Symbols.drag_indicator,
                                    title: item.title,
                                    subtitle: item.subtitle,
                                    onPressed: () {},
                                  ),
                          )
                          .toList(),
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
