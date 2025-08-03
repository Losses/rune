import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/ax_shadow.dart';
import '../../../utils/format_time.dart';
import '../../../utils/playing_item.dart';
import '../../../utils/color_brightness.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/navigation_bar/page_content_frame.dart';
import '../../../widgets/cover_wall_background/gradient_container.dart';
import '../../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../bindings/bindings.dart';
import '../../../providers/status.dart';
import '../../../providers/responsive_providers.dart';

import '../../cover_wall/widgets/cover_wall_layout.dart';

import 'lyric_display.dart';

class LyricsLayout extends StatefulWidget {
  final PlayingItem? item;
  final List<LyricContentLine> lyrics;
  final int currentTimeMilliseconds;
  final List<int> activeLines;
  const LyricsLayout({
    super.key,
    required this.item,
    required this.lyrics,
    required this.currentTimeMilliseconds,
    required this.activeLines,
  });

  @override
  LyricsLayoutState createState() => LyricsLayoutState();
}

class LyricsLayoutState extends State<LyricsLayout> {
  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor;

    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.tablet, false);

    return LayoutBuilder(
      builder: (context, constraints) {
        final gridSize = calculateCoverWallGridSize(constraints);
        final crossAxisCount = (constraints.maxWidth / gridSize).ceil();
        final mainAxisCount = (constraints.maxHeight / gridSize).ceil();

        final coverArtWall = ClipRect(
          child: OverflowBox(
            alignment: Alignment.topLeft,
            maxWidth: (crossAxisCount * gridSize).toDouble(),
            maxHeight: (mainAxisCount * gridSize).toDouble(),
            child: Center(
              child: SizedBox.expand(
                child: GradientContainer(
                  gradientParams: gradientParams,
                  effectParams: effectParams,
                  color: isDark ? theme.accentColor : theme.accentColor.darkest,
                  color2: theme.accentColor.darkest.darken(0.7),
                  child: const CoverWallBackground(seed: 42, gap: 2),
                ),
              ),
            ),
          ),
        );

        return Stack(
          alignment: Alignment.bottomCenter,
          children: [
            Container(
              color: isDark
                  ? Colors.black
                  : theme.accentColor.lightest.lighten(0.2),
            ),
            coverArtWall,
            Container(
              decoration: BoxDecoration(
                gradient: RadialGradient(
                  colors: [
                    shadowColor.withAlpha(isDark ? 170 : 190),
                    shadowColor.withAlpha(isDark ? 255 : 255),
                  ],
                  radius: 1.5,
                ),
              ),
              height: (mainAxisCount * gridSize).toDouble(),
            ),
            if (!isMini && !isDark)
              Container(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: const Alignment(0.0, -1.0),
                    end: const Alignment(0.0, 1.0),
                    colors: [
                      shadowColor.withAlpha(0),
                      shadowColor.lighten(0.8).withAlpha(220),
                    ],
                  ),
                ),
                height: constraints.maxHeight * 0.6,
              ),
            if (!isMini)
              Positioned(
                top: 0,
                left: 0,
                child: SizedBox(
                  width: constraints.maxWidth / 5 * 2,
                  height: constraints.maxHeight,
                  child: Padding(
                    padding: EdgeInsets.only(right: 36),
                    child: Align(
                      alignment: Alignment.centerRight,
                      child: CoverArtFrame(),
                    ),
                  ),
                ),
              ),
            Positioned(
              top: 0,
              right: 0,
              child: SizedBox(
                width: isMini
                    ? constraints.maxWidth
                    : constraints.maxWidth / 5 * 3,
                height: constraints.maxHeight,
                child: PageContentFrame(
                  child: LyricsDisplay(
                    key: ValueKey(widget.item),
                    lyrics: widget.lyrics,
                    currentTimeMilliseconds: widget.currentTimeMilliseconds,
                    activeLines: widget.activeLines,
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

class CoverArtFrame extends StatelessWidget {
  const CoverArtFrame({super.key});

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: Colors.white, width: 4),
        boxShadow: axShadow(9),
      ),
      child: Selector<PlaybackStatusProvider, (String, String, String, double)>(
        selector: (context, playbackStatusProvider) {
          final s = playbackStatusProvider.playbackStatus;

          return (
            s.coverArtPath ?? "",
            s.album ?? "",
            s.artist ?? "",
            s.duration
          );
        },
        builder: (context, p, child) {
          return CoverArt(
            key: p.$1.isNotEmpty ? Key(p.toString()) : null,
            path: p.$1,
            hint: (p.$2, p.$3, 'Total Time ${formatTime(p.$4)}'),
            size: 220,
          );
        },
      ),
    );
  }
}
