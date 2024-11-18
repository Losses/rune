import 'dart:math';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/nearest_power_of_two.dart';
import '../../../utils/load_and_resize_image.dart';
import '../../../screens/cover_wall/utils/string_to_double.dart';

import '../utils/random_grid_placement.dart';
import '../utils/calculate_cover_wall_size.dart';
import '../utils/cover_wall_background_painter.dart';
import '../constants/random_grid_config.dart';
import '../constants/max_random_grid_config_size.dart';

class CoverWallBackgroundImplementation extends StatefulWidget {
  final int seed;
  final int gap;
  final List<String> paths;
  final BoxConstraints constraints;

  const CoverWallBackgroundImplementation({
    super.key,
    required this.seed,
    required this.gap,
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

  int size = 0;

  _loadAllImages() {
    final nextSize = nearestPowerOfTwo(
      calculateCoverWallSize(widget.constraints).ceil() *
          maxRandomGridConfigSize *
          pixelRatio.ceil(),
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

    final gridSize = calculateCoverWallSize(constraints);
    final cols =
        (constraints.maxWidth / gridSize).ceil() + maxRandomGridConfigSize;
    final rows =
        (constraints.maxHeight / gridSize).ceil() + maxRandomGridConfigSize;

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
    final gridSize = calculateCoverWallSize(widget.constraints);

    return CustomPaint(
      painter: CoverWallBackgroundPainter(
        gridSize: gridSize.ceil(),
        gap: widget.gap,
        images: images,
        grid: grid,
      ),
    );
  }
}
