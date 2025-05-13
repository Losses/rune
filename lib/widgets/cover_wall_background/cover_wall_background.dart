import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/rune_log.dart';
import '../../utils/query_list.dart';
import '../../utils/settings_manager.dart';
import '../../utils/nearest_power_of_two.dart';
import '../../utils/load_and_resize_image.dart';
import '../../utils/process_cover_art_path.dart';
import '../../utils/api/query_mix_tracks.dart';
import '../../constants/configurations.dart';

import 'utils/generate_tiles_of_size.dart';
import 'utils/calculate_cover_wall_size.dart';
import 'utils/cover_wall_background_painter.dart';
import 'constants/max_random_grid_config_size.dart';

const coverCount = '40';

class CoverWallBackground extends StatefulWidget {
  final int seed;
  final int gap;

  const CoverWallBackground({
    super.key,
    required this.seed,
    required this.gap,
  });

  @override
  State<CoverWallBackground> createState() => _CoverWallBackgroundState();
}

class _CoverWallBackgroundState extends State<CoverWallBackground> {
  Set<String> paths = {};
  List<ui.Image?> images = [];
  int size = 0;
  late double pixelRatio;

  @override
  void initState() {
    super.initState();
    loadCoverList();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    pixelRatio = MediaQuery.devicePixelRatioOf(context);
    _loadAllImages();
  }

  @override
  void didUpdateWidget(CoverWallBackground oldWidget) {
    super.didUpdateWidget(oldWidget);
    _loadAllImages();
  }

  @override
  void dispose() {
    super.dispose();
    for (int i = 0; i < images.length; i += 1) {
      images[i] = null;
    }
  }

  loadCoverList() async {
    final String count =
        await SettingsManager().getValue<String?>(kRandomCoverWallCountKey) ??
            coverCount;

    final queryResult = await queryMixTracks(
      QueryList([
        ("lib::random", count.toString()),
        ("filter::with_cover_art", "true"),
      ]),
      0,
      int.parse(count),
    );

    if (!mounted) return;

    setState(() {
      paths = queryResult.map((x) => x.coverArtPath).toSet();
      images = List.filled(paths.length, null);
    });

    _loadAllImages();
  }

  _loadAllImages() {
    if (paths.isEmpty || !context.mounted) return;

    // We need BoxConstraints for this calculation
    final box = context.findRenderObject() as RenderBox?;
    if (box == null) return;

    final constraints = BoxConstraints(
      maxWidth: box.size.width,
      maxHeight: box.size.height,
    );

    final nextSize = nearestPowerOfTwo(
      calculateCoverWallGridSize(constraints).ceil() *
          maxRandomGridConfigSize *
          pixelRatio.ceil(),
    );

    if (size == nextSize) return;
    size = nextSize;

    final pathsList = paths.toList();
    for (int i = 0; i < pathsList.length; i += 1) {
      final path = pathsList[i];

      // First process the cover art path to handle remote URLs
      processCoverArtPath(path).then((processedPath) {
        // Then load and resize the image from the processed path
        loadAndResizeImage(processedPath, nextSize).then((image) {
          if (!mounted) return;

          setState(() {
            images[i] = image;
          });
        }).catchError((error) {
          error$('Error loading image: $error');
        });
      }).catchError((error) {
        error$('Error processing cover art path: $error');
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (paths.isEmpty) {
      return Container();
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        final grid = generateTilesOfSize(
          constraints,
          paths.length,
          widget.seed,
          size,
        );
        final gridSize = calculateCoverWallGridSize(constraints);

        return CustomPaint(
          painter: CoverWallBackgroundPainter(
            gridSize: gridSize.ceil(),
            gap: widget.gap,
            images: images,
            grid: grid,
          ),
        );
      },
    );
  }
}
