import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/color_brightness.dart';
import '../../../widgets/cover_wall_background/gradient_container.dart';
import '../../../widgets/cover_wall_background/cover_wall_background.dart';
import '../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../providers/responsive_providers.dart';

import 'back_button.dart';
import 'playing_track.dart';

class CoverWallLayout extends StatefulWidget {
  const CoverWallLayout({super.key});

  @override
  CoverWallLayoutState createState() => CoverWallLayoutState();
}

const gradientParams = GradientParams(
  multX: 2.0,
  multY: 2.0,
  brightness: 1.0,
);

const effectParams = EffectParams(
  mouseInfluence: -0.2,
  scale: 1.25,
  noise: 1.5,
  bw: 0.0,
);

class CoverWallLayoutState extends State<CoverWallLayout> {
  @override
  void initState() {
    super.initState();
  }

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
              color: isDark ? Colors.black : theme.accentColor.lightest.lighten(0.2),
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
            const PlayingTrack(),
            if (!isMini)
              Container(
                decoration: BoxDecoration(
                  gradient: LinearGradient(
                    begin: const Alignment(0.0, -1.0),
                    end: const Alignment(0.0, 1.0),
                    colors: [
                      shadowColor.withAlpha(0),
                      isDark
                          ? shadowColor.withAlpha(200)
                          : shadowColor.lighten(0.2).withAlpha(220),
                    ],
                  ),
                ),
                height: playbackControllerHeight,
              ),
            const Positioned(
              top: 0,
              left: 0,
              child: BackButton(),
            )
          ],
        );
      },
    );
  }
}
