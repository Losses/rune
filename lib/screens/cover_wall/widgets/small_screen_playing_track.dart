import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/providers/playback_controller.dart';
import 'package:rune/providers/volume.dart';
import 'package:rune/widgets/playback_controller/constants/controller_items.dart';

import '../../../utils/ax_shadow.dart';
import '../../../utils/format_time.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../screens/cover_wall/widgets/cover_art_page_progress_bar.dart';
import '../../../providers/status.dart';
import '../../../providers/responsive_providers.dart';

class SmallScreenPlayingTrack extends StatelessWidget {
  const SmallScreenPlayingTrack({super.key});

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    final typography = theme.typography;

    final shadows = <Shadow>[
      Shadow(color: shadowColor, blurRadius: 12),
      Shadow(color: shadowColor, blurRadius: 24),
    ];

    final width = MediaQuery.of(context).size.width;

    Provider.of<VolumeProvider>(context);

    return Selector<PlaybackStatusProvider,
        (String?, String?, String?, String?, double?, String?)>(
      selector: (context, playbackStatusProvider) => (
        playbackStatusProvider.playbackStatus?.coverArtPath,
        playbackStatusProvider.playbackStatus?.artist,
        playbackStatusProvider.playbackStatus?.album,
        playbackStatusProvider.playbackStatus?.title,
        playbackStatusProvider.playbackStatus?.duration,
        playbackStatusProvider.playbackStatus?.state,
      ),
      builder: (context, p, child) {
        if (p.$1 == null) return Container();

        final artist = p.$2 ?? "Unknown Artist";
        final album = p.$3 ?? "Unknown Album";

        return Container(
          padding: const EdgeInsets.fromLTRB(
            12,
            12,
            12,
            playbackControllerHeight + 12,
          ),
          constraints: const BoxConstraints(maxWidth: 240),
          child: SmallerOrEqualTo(
            breakpoint: DeviceType.zune,
            builder: (context, isZune) {
              return Column(
                crossAxisAlignment: CrossAxisAlignment.center,
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  if (!isZune)
                    Container(
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      child: Container(
                        decoration: BoxDecoration(
                          border: Border.all(color: Colors.white, width: 4),
                          boxShadow: axShadow(9),
                        ),
                        child: AspectRatio(
                          aspectRatio: 1,
                          child: CoverArt(
                            hint: (
                              p.$3 ?? "",
                              p.$2 ?? "",
                              'Total Time ${formatTime(p.$5 ?? 0)}'
                            ),
                            key: p.$1 != null ? Key(p.$1.toString()) : null,
                            path: p.$1,
                            size: (width - 20).clamp(0, 240),
                          ),
                        ),
                      ),
                    ),
                  if (!isZune)
                    Transform.translate(
                      offset: const Offset(0, -16),
                      child: SizedBox(
                        height: 80,
                        child: CoverArtPageProgressBar(shadows: shadows),
                      ),
                    ),
                  const SizedBox(height: 8),
                  Text(
                    p.$4 ?? "Unknown Track",
                    style: typography.subtitle?.apply(shadows: shadows),
                    textAlign: TextAlign.center,
                  ),
                  const SizedBox(height: 12),
                  Text(
                    '$artist · $album',
                    style: typography.body
                        ?.apply(shadows: shadows, heightFactor: 2),
                    textAlign: TextAlign.center,
                  ),
                  if (isZune) const SizedBox(height: 12),
                  if (isZune)
                    Selector<PlaybackControllerProvider,
                        (List<ControllerEntry>, List<ControllerEntry>)>(
                      selector: (context, controllerProvider) {
                        final entries = controllerProvider.entries;
                        final hiddenIndex =
                            entries.indexWhere((entry) => entry.id == 'hidden');
                        final List<ControllerEntry> visibleEntries =
                            hiddenIndex != -1
                                ? entries.sublist(0, hiddenIndex)
                                : entries;
                        final List<ControllerEntry> hiddenEntries =
                            hiddenIndex != -1
                                ? entries.sublist(hiddenIndex + 1)
                                : [];

                        return (visibleEntries, hiddenEntries);
                      },
                      builder: (context, entries, child) {
                        return CommandBar(
                          isCompact: true,
                          overflowMenuItemBuilder: (context, entry) {
                            if (entry is PrimaryCommandBarItem) {
                              return entry.entry.flyoutEntryBuilder(context);
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
                                ),
                              )
                              .toList(),
                          secondaryItems: entries.$2
                              .map(
                                (x) => PrimaryCommandBarItem(
                                  key: ValueKey(x.id),
                                  entry: x,
                                ),
                              )
                              .toList(),
                        );
                      },
                    )
                ],
              );
            },
          ),
        );
      },
    );
  }
}

class PrimaryCommandBarItem extends CommandBarItem {
  PrimaryCommandBarItem({required super.key, required this.entry});

  final ControllerEntry entry;

  @override
  Widget build(BuildContext context, CommandBarItemDisplayMode displayMode) {
    return entry.controllerButtonBuilder(context);
  }
}

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
