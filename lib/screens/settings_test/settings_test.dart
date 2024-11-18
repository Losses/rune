import 'dart:math';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../utils/draw_image_to_canvas.dart';
import '../../utils/nearest_power_of_two.dart';
import '../../utils/load_and_resize_image.dart';

import '../cover_wall/utils/string_to_double.dart';
import '../cover_wall/utils/random_grid_config.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return CoverWallBackground(
      seed: 114514,
    );
  }
}

class RandomGridPlacement {
  final int coverIndex;
  final int col;
  final int row;
  final int size;

  const RandomGridPlacement({
    required this.coverIndex,
    required this.col,
    required this.row,
    required this.size,
  });

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other is! RandomGridPlacement) return false;
    return coverIndex == other.coverIndex &&
        col == other.col &&
        row == other.row &&
        size == other.size;
  }

  @override
  int get hashCode => Object.hash(coverIndex, col, row, size);

  @override
  String toString() =>
      'RandomGridPlacement(coverIndex: $coverIndex, col: $col, row: $row, size: $size)';
}

const randomGridConfig = [
  RandomGridConfig(size: 4, probability: 0.2),
  RandomGridConfig(size: 3, probability: 0.3),
  RandomGridConfig(size: 2, probability: 0.3),
];

const maxRandomGridConfigSize = 4;

class CoverWallBackground extends StatefulWidget {
  final int seed;

  const CoverWallBackground({
    super.key,
    required this.seed,
  });

  @override
  State<CoverWallBackground> createState() => _CoverWallBackgroundState();
}

class _CoverWallBackgroundState extends State<CoverWallBackground> {
  final List<String> paths = [];

  @override
  void initState() {
    super.initState();
    loadCoverList();
  }

  loadCoverList() async {
    final queryResult = await queryMixTracks(
      QueryList([
        ("lib::random", "30"),
        ("filter::with_cover_art", "true"),
      ]),
    );

    setState(() {
      for (final file in queryResult) {
        paths.add(file.coverArtPath);
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    if (paths.isEmpty) {
      return Container();
    }

    return LayoutBuilder(
      builder: (context, constraints) => CoverWallBackgroundImplementation(
        seed: widget.seed,
        paths: paths,
        constraints: constraints,
      ),
    );
  }
}

class CoverWallBackgroundImplementation extends StatefulWidget {
  final int seed;
  final List<String> paths;
  final BoxConstraints constraints;

  const CoverWallBackgroundImplementation({
    super.key,
    required this.seed,
    required this.paths,
    required this.constraints,
  });

  @override
  CoverWallBackgroundImplementationState createState() =>
      CoverWallBackgroundImplementationState();
}

class CoverWallBackgroundImplementationState
    extends State<CoverWallBackgroundImplementation> {
  late List<ui.Image?> images = List.filled(widget.paths.length, null);

  late double pixelRatio;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    pixelRatio = MediaQuery.devicePixelRatioOf(context);

    _loadAllImages();
  }

  @override
  didUpdateWidget(oldWidget) {
    super.didUpdateWidget(oldWidget);

    _loadAllImages();
  }

  double calculateGridSize() {
    return max(
      max(widget.constraints.maxWidth, widget.constraints.maxHeight) / 24,
      64,
    );
  }

  int size = 0;

  _loadAllImages() {
    final nextSize = nearestPowerOfTwo(
      calculateGridSize().ceil() * maxRandomGridConfigSize * pixelRatio.ceil(),
    );

    if (size == nextSize) return;
    size = nextSize;

    for (int i = 0; i < widget.paths.length; i += 1) {
      final path = widget.paths[i];

      loadAndResizeImage(path, nextSize).then((image) {
        setState(() {
          images[i] = image;
        });
      });
    }
  }

  List<List<RandomGridPlacement>> _generateTilesOfSize() {
    final constraints = widget.constraints;

    final gridSize = calculateGridSize();
    final cols = (constraints.maxWidth / gridSize).ceil();
    final rows = (constraints.maxHeight / gridSize).ceil();

    final totalGrids = cols * rows;

    final List<bool> occupied = List.filled(totalGrids, false);
    final List<List<RandomGridPlacement>> placement =
        List.generate(widget.paths.length, (_) => [], growable: false);

    for (int i = 0; i < occupied.length; i += 1) {
      final int row = i ~/ cols;
      final int col = i % cols;

      final gridKey = '$row-$col';

      if (occupied[i]) continue;

      double randomValue1 = stringToDouble('$gridKey-${widget.seed}');
      double randomValue2 = stringToDouble('$gridKey-i-${widget.seed}');
      int coverIndex = (randomValue2 * (widget.paths.length - 1)).round();

      int maxSize = maxRandomGridConfigSize;

      for (int colP = 0; colP < maxSize; colP++) {
        for (int rowP = 0; rowP < maxSize; rowP++) {
          if ((col + colP) >= cols) continue;
          if ((row + rowP) >= rows) continue;

          final index = (col + colP) + (row + rowP) * cols;

          if (occupied[index]) {
            maxSize = min(colP, rowP);
          }
        }
      }

      if (maxSize == 0) continue;

      for (final cfg in randomGridConfig) {
        if (size < maxSize) continue;

        if (randomValue1 <= cfg.probability) {
          final size = cfg.size;

          for (int colP = 0; colP < size; colP++) {
            for (int rowP = 0; rowP < size; rowP++) {
              if (col + colP >= cols) continue;
              if (row + rowP >= rows) continue;

              final indexA = (col + colP) + (row + rowP) * cols;

              if (indexA < occupied.length && col <= cols) {
                occupied[indexA] = true;
              }
            }
          }

          placement[coverIndex].add(
            RandomGridPlacement(
              coverIndex: coverIndex,
              col: col,
              row: row,
              size: size,
            ),
          );

          break;
        }
      }
    }

    for (int i = 0; i < occupied.length; i += 1) {
      if (occupied[i]) continue;

      final int row = i ~/ cols;
      final int col = i % cols;

      final gridKey = '$row-$col';
      double randomValue2 = stringToDouble('$gridKey-i-${widget.seed}');
      int coverIndex = (randomValue2 * (widget.paths.length - 1)).round();

      occupied[i] = true;

      placement[coverIndex].add(
        RandomGridPlacement(
          coverIndex: coverIndex,
          col: col,
          row: row,
          size: 1,
        ),
      );
    }

    return placement;
  }

  @override
  Widget build(BuildContext context) {
    final grid = _generateTilesOfSize();
    final gridSize = calculateGridSize();

    return CustomPaint(
      painter: CoverWallBackgroundPainter(
        gridSize: gridSize.ceil(),
        images: images,
        grid: grid,
      ),
    );
  }
}

class CoverWallBackgroundPainter extends CustomPainter {
  final int gridSize;
  final List<ui.Image?> images;
  final List<List<RandomGridPlacement>> grid;

  CoverWallBackgroundPainter({
    required this.gridSize,
    required this.images,
    required this.grid,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint();

    for (int imageIndex = 0; imageIndex < grid.length; imageIndex += 1) {
      final image = images[imageIndex];

      if (image == null) return;

      for (final gridUnit in grid[imageIndex]) {
        canvas.save();
        canvas.translate(
          (gridSize * gridUnit.col + gridSize * (gridUnit.size / 2)).toDouble(),
          (gridSize * gridUnit.row + gridSize * (gridUnit.size / 2)).toDouble(),
        );

        drawImageToCanvas(
          canvas,
          paint,
          image,
          gridSize * gridUnit.size,
          gridSize * gridUnit.size,
        );
        canvas.restore();
      }
    }
  }

  @override
  bool shouldRepaint(CustomPainter oldDelegate) => true;
}
