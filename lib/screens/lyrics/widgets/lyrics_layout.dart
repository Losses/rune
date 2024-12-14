import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/color_brightness.dart';
import '../../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../messages/all.dart';
import '../../../providers/responsive_providers.dart';

import '../../cover_wall/widgets/cover_wall_layout.dart';
import '../../cover_wall/widgets/gradient_container.dart';

import 'lyric_display.dart';

class LyricsLayout extends StatefulWidget {
  final List<LyricContentLine> lyrics;
  final int currentTimeMilliseconds;
  final List<int> activeLines;
  const LyricsLayout({
    super.key,
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
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    final r = Provider.of<ResponsiveProvider>(context);
    final isMini = r.smallerOrEqualTo(DeviceType.car, false);

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
          alignment: isMini ? Alignment.centerLeft : Alignment.bottomCenter,
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
                    shadowColor.withAlpha(isDark ? 20 : 140),
                    shadowColor.withAlpha(isDark ? 255 : 255),
                  ],
                  radius: 1.5,
                ),
              ),
              height: (mainAxisCount * gridSize).toDouble(),
            ),
            LyricsDisplay(
              lyrics: widget.lyrics,
              currentTimeMilliseconds: widget.currentTimeMilliseconds,
              activeLines: widget.activeLines,
            )
          ],
        );
      },
    );
  }
}
