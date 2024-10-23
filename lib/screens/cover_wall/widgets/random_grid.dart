import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';

import '../../../utils/color_brightness.dart';
import '../../../widgets/tile/cover_art.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';

import '../utils/string_to_double.dart';
import '../utils/random_grid_config.dart';

import 'grid_tile.dart';
import 'back_button.dart';
import 'playing_track.dart';
import 'gradient_container.dart';

class RandomGrid extends StatefulWidget {
  final int seed;
  final List<String> paths;
  const RandomGrid({super.key, required this.seed, required this.paths});

  @override
  RandomGridState createState() => RandomGridState();
}

class RandomGridState extends State<RandomGrid> {
  @override
  void initState() {
    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final isDark = theme.brightness.isDark;
    final shadowColor = isDark ? Colors.black : theme.accentColor.lightest;

    return LayoutBuilder(
      builder: (context, constraints) {
        final gridSize =
            max(max(constraints.maxWidth, constraints.maxHeight) / 24, 64);
        final crossAxisCount = (constraints.maxWidth / gridSize).ceil();
        final mainAxisCount = (constraints.maxHeight / gridSize).ceil();

        final coverArtWall = widget.paths.isEmpty
            ? Container(
                color: shadowColor,
              )
            : ClipRect(
                child: OverflowBox(
                  alignment: Alignment.topLeft,
                  maxWidth: (crossAxisCount * gridSize).toDouble(),
                  maxHeight: (mainAxisCount * gridSize).toDouble(),
                  child: Center(
                    child: GradientContainer(
                      gradientParams: GradientParams(
                        multX: 2.0,
                        multY: 2.0,
                        brightness: 1.0,
                      ),
                      effectParams: EffectParams(
                        mouseInfluence: -0.2,
                        scale: 1.25,
                        noise: 1.5,
                        bw: 0.0,
                      ),
                      color: isDark
                          ? theme.accentColor
                          : theme.accentColor.darkest,
                      color2: theme.accentColor.darkest.darken(0.7),
                      child: StaggeredGrid.count(
                        crossAxisCount: crossAxisCount,
                        mainAxisSpacing: 2,
                        crossAxisSpacing: 2,
                        children: _generateTiles(
                          crossAxisCount,
                          mainAxisCount,
                          gridSize.toDouble(),
                        ),
                      ),
                    ),
                  ),
                ),
              );

        return Stack(
          alignment: Alignment.bottomCenter,
          children: [
            Container(
              color: isDark ? null : theme.accentColor.lightest.lighten(0.2),
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
                height: (mainAxisCount * gridSize).toDouble()),
            const PlayingTrack(),
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

  List<Widget> _generateTiles(
      int crossAxisCount, int mainAxisCount, double gridSize) {
    List<Widget> tiles = [];
    Set<String> occupiedCells = {};

    // Step 1: Generate 4x4 tiles
    _generateTilesOfSize(
        tiles,
        occupiedCells,
        gridSize,
        [
          const RandomGridConfig(size: 4, probability: 0.2),
          const RandomGridConfig(size: 3, probability: 0.3),
          const RandomGridConfig(size: 2, probability: 0.3),
        ],
        crossAxisCount,
        mainAxisCount);
    return tiles;
  }

  void _generateTilesOfSize(
    List<Widget> tiles,
    Set<String> occupiedCells,
    double gridSize,
    List<RandomGridConfig> config,
    int crossAxisCount,
    int mainAxisCount,
  ) {
    for (int row = 0; row < mainAxisCount; row++) {
      for (int col = 0; col < crossAxisCount; col++) {
        final gridKey = '$col-$row';

        if (occupiedCells.contains(gridKey)) {
          continue;
        }

        double randomValue1 = stringToDouble('$gridKey-${widget.seed}');
        double randomValue2 = stringToDouble('$gridKey-i-${widget.seed}');
        int coverIndex = (randomValue2 * (widget.paths.length - 1)).round();

        for (var cfg in config) {
          if (randomValue1 <= cfg.probability) {
            int size = cfg.size;

            if (_canPlaceTile(
                col, row, size, crossAxisCount, mainAxisCount, occupiedCells)) {
              _markOccupiedCells(col, row, size, occupiedCells);
              tiles.add(
                StaggeredGridTile.count(
                  crossAxisCellCount: size,
                  mainAxisCellCount: size,
                  child: GridTile(
                    index: row + col * mainAxisCount,
                    row: row,
                    col: col,
                    size: size,
                    child: CoverArt(
                      path: widget.paths[coverIndex],
                      size: size * gridSize,
                    ),
                  ),
                ),
              );
              break; // Once a tile is placed, move to the next cell
            } else if (_canPlaceTile(
                col, row, 1, crossAxisCount, mainAxisCount, occupiedCells)) {
              _markOccupiedCells(col, row, 1, occupiedCells);

              tiles.add(
                StaggeredGridTile.count(
                  crossAxisCellCount: 1,
                  mainAxisCellCount: 1,
                  child: GridTile(
                    index: coverIndex,
                    row: row,
                    col: col,
                    size: 1,
                    child: CoverArt(
                      path: widget.paths[coverIndex],
                      size: 1 * gridSize,
                    ),
                  ),
                ),
              );
            }
          }
        }

        if (_canPlaceTile(
            col, row, 1, crossAxisCount, mainAxisCount, occupiedCells)) {
          _markOccupiedCells(col, row, 1, occupiedCells);
          tiles.add(
            StaggeredGridTile.count(
              crossAxisCellCount: 1,
              mainAxisCellCount: 1,
              child: GridTile(
                  index: coverIndex,
                  row: row,
                  col: col,
                  size: 1,
                  child: CoverArt(
                    path: widget.paths[coverIndex],
                    size: 64.0,
                  )),
            ),
          );
        }
      }
    }
  }

  bool _canPlaceTile(int col, int row, int size, int crossAxisCount,
      int mainAxisCount, Set<String> occupiedCells) {
    for (int i = 0; i < size; i++) {
      for (int j = 0; j < size; j++) {
        if (col + i >= crossAxisCount ||
            row + j >= mainAxisCount ||
            occupiedCells.contains('${col + i}-${row + j}')) {
          return false;
        }
      }
    }
    return true;
  }

  void _markOccupiedCells(
      int col, int row, int size, Set<String> occupiedCells) {
    for (int i = 0; i < size; i++) {
      for (int j = 0; j < size; j++) {
        occupiedCells.add('${col + i}-${row + j}');
      }
    }
  }
}
