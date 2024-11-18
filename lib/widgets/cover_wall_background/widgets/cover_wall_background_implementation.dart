import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/nearest_power_of_two.dart';
import '../../../utils/load_and_resize_image.dart';

import '../utils/generate_tiles_of_size.dart';
import '../utils/calculate_cover_wall_size.dart';
import '../utils/cover_wall_background_painter.dart';
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

  @override
  Widget build(BuildContext context) {
    final grid = generateTilesOfSize(
      widget.constraints,
      widget.paths.length,
      widget.seed,
      size,
    );
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
