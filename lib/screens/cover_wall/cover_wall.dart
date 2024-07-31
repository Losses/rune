import 'dart:math';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';
import 'package:fluent_ui/fluent_ui.dart';

class RandomGridConfig {
  final int size;
  final double probability;

  const RandomGridConfig({required this.size, required this.probability});
}

class RandomGrid extends StatefulWidget {
  final int seed;
  const RandomGrid({super.key, required this.seed});

  @override
  RandomGridState createState() => RandomGridState();
}

class RandomGridState extends State<RandomGrid> {
  late Random _random;

  @override
  void initState() {
    super.initState();
    _random = Random(widget.seed);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final gridSize = max(constraints.maxWidth, constraints.maxHeight) / 24;
        final crossAxisCount = (constraints.maxWidth / gridSize).floor();
        final mainAxisCount = (constraints.maxHeight / gridSize).floor();

        return StaggeredGrid.count(
          crossAxisCount: crossAxisCount,
          mainAxisSpacing: 2,
          crossAxisSpacing: 2,
          children: _generateTiles(crossAxisCount, mainAxisCount),
        );
      },
    );
  }

  List<Widget> _generateTiles(int crossAxisCount, int mainAxisCount) {
    List<Widget> tiles = [];
    Set<String> occupiedCells = {};

    // Step 1: Generate 4x4 tiles
    _generateTilesOfSize(
        tiles,
        occupiedCells,
        [
          const RandomGridConfig(size: 3, probability: 0.2),
          const RandomGridConfig(size: 4, probability: 0.4),
        ],
        crossAxisCount,
        mainAxisCount);
    return tiles;
  }

  void _generateTilesOfSize(
    List<Widget> tiles,
    Set<String> occupiedCells,
    List<RandomGridConfig> config,
    int crossAxisCount,
    int mainAxisCount,
  ) {
    for (int row = 0; row < mainAxisCount; row++) {
      for (int col = 0; col < crossAxisCount; col++) {
        if (occupiedCells.contains('$col-$row')) {
          continue;
        }

        double randomValue = _random.nextDouble();

        for (var cfg in config) {
          if (randomValue <= cfg.probability) {
            int size = cfg.size;

            if (_canPlaceTile(
                col, row, size, crossAxisCount, mainAxisCount, occupiedCells)) {
              _markOccupiedCells(col, row, size, occupiedCells);
              tiles.add(
                StaggeredGridTile.count(
                  crossAxisCellCount: size,
                  mainAxisCellCount: size,
                  child: GridTile(
                      index: _random.nextInt(30),
                      row: row,
                      col: col,
                      size: size,
                      child: Text("$col x $row")),
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
                      index: _random.nextInt(30),
                      row: row,
                      col: col,
                      size: 1,
                      child: Text("$col x $row")),
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
                  index: _random.nextInt(30),
                  row: row,
                  col: col,
                  size: 1,
                  child: Text("$col x $row")),
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

class GridTile extends StatelessWidget {
  final int index;
  final int row;
  final int col;
  final int size;
  final Widget child;

  const GridTile(
      {super.key,
      required this.index,
      required this.row,
      required this.col,
      required this.size,
      required this.child});

  @override
  Widget build(BuildContext context) {
    return Container(
      color: FluentTheme.of(context).accentColor,
      child: Center(
        child: child,
      ),
    );
  }
}

class CoverWall extends StatefulWidget {
  const CoverWall({super.key});

  @override
  State<CoverWall> createState() => _CoverWallState();
}

class _CoverWallState extends State<CoverWall> {
  @override
  Widget build(BuildContext context) {
    return const ScaffoldPage(
      content: Center(
        child: RandomGrid(seed: 42),
      ),
    );
  }
}
