import 'dart:ui';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:reorderables/reorderables.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/l10n.dart';
import '../../utils/settings_page_padding.dart';
import '../../utils/settings_body_padding.dart';
import '../../utils/build_draggable_feedback.dart';
import '../../widgets/pointer_type.dart';
import '../../widgets/unavailable_page_on_band.dart';
import '../../widgets/settings/settings_button.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
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
            padding: getScrollContainerPadding(context),
            child: SettingsBodyPadding(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                    width: double.maxFinite,
                    child: PointerTypeBuilder(
                      builder: (kind) {
                        return ReorderableColumn(
                          needsLongPressDraggable:
                              kind == PointerDeviceKind.touch,
                          onReorder: playbackController.reorder,
                          buildDraggableFeedback: buildDraggableFeedback,
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
                                              padding:
                                                  const EdgeInsets.symmetric(
                                                horizontal: 24,
                                              ),
                                              child: Text(
                                                S.of(context).actionMenu,
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
                                        title: item.titleBuilder(context),
                                        subtitle: item.subtitleBuilder(context),
                                        onPressed: () {},
                                      ),
                              )
                              .toList(),
                        );
                      },
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
