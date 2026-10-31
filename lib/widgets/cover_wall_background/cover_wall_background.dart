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
  @visibleForTesting
  final Future<List<String>> Function()? loadPaths;
  @visibleForTesting
  final Future<ui.Image> Function(String, int)? loadImage;

  const CoverWallBackground({
    super.key,
    required this.seed,
    required this.gap,
    this.loadPaths,
    this.loadImage,
  });

  @override
  State<CoverWallBackground> createState() => _CoverWallBackgroundState();
}

class _CoverWallBackgroundState extends State<CoverWallBackground> {
  Set<String> paths = {};
  List<ui.Image?> images = [];
  int size = 0;
  late double pixelRatio;
  int _loadGeneration = 0;

  @override
  void initState() {
    super.initState();
    loadCoverList();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    pixelRatio = MediaQuery.devicePixelRatioOf(context);
  }

  @override
  void didUpdateWidget(CoverWallBackground oldWidget) {
    super.didUpdateWidget(oldWidget);
    _loadAllImages();
  }

  @override
  void dispose() {
    for (final image in images) {
      image?.dispose();
    }
    super.dispose();
  }

  loadCoverList() async {
    final List<String> coverPaths;
    if (widget.loadPaths != null) {
      coverPaths = await widget.loadPaths!();
    } else {
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
      coverPaths = queryResult.map((x) => x.coverArtPath).toList();
    }
    if (!mounted) return;

    setState(() {
      paths = coverPaths.toSet();
      images = List.filled(paths.length, null);
    });

    _loadAllImages();
  }

  _loadAllImages() {
    if (paths.isEmpty || !context.mounted) return;

    // We need BoxConstraints for this calculation
    final box = context.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize) return;

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
    final generation = ++_loadGeneration;

    final pathsList = paths.toList();
    for (int i = 0; i < pathsList.length; i += 1) {
      final path = pathsList[i];

      Future<ui.Image> load() async {
        if (widget.loadImage != null) return widget.loadImage!(path, nextSize);
        final processedPath = await processCoverArtPath(path);
        return loadAndResizeImage(processedPath, nextSize);
      }

      load()
          .then((image) {
            if (!mounted || generation != _loadGeneration) {
              image.dispose();
              return;
            }
            final previous = images[i];
            setState(() {
              images = List.of(images)..[i] = image;
            });
            previous?.dispose();
          })
          .catchError((error) {
            error$('Error loading image: $error');
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
        final requiredSize = nearestPowerOfTwo(
          calculateCoverWallGridSize(constraints).ceil() *
              maxRandomGridConfigSize *
              pixelRatio.ceil(),
        );
        if (requiredSize != size) {
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) _loadAllImages();
          });
        }
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
