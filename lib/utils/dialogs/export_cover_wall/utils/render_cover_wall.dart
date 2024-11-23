import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/utils/query_list.dart';

import '../../../../messages/all.dart';
import '../../../../widgets/cover_wall_background/utils/calculate_cover_wall_size.dart';
import '../../../../widgets/cover_wall_background/utils/generate_tiles_of_size.dart';
import '../../../../widgets/cover_wall_background/utils/cover_wall_background_painter.dart';
import '../../../../widgets/cover_wall_background/constants/max_random_grid_config_size.dart';

import '../../../build_query.dart';
import '../../../load_and_resize_image.dart';
import '../../../api/query_mix_tracks.dart';

Future<ui.Image> renderCoverWall(
  CollectionType type,
  int id,
) async {
  const sizeDefinition = BoxConstraints(maxWidth: 1920, maxHeight: 1080);
  final gridSize = calculateCoverWallGridSize(sizeDefinition).ceil();
  const gap = 4;

  ui.PictureRecorder recorder = ui.PictureRecorder();
  ui.Canvas canvas = ui.Canvas(recorder);
  final queries = await buildQuery(type, id);
  final newItems = await queryMixTracks(
    QueryList([...queries, ('filter::with_cover_art', 'true')]),
    0,
    999,
  );

  final paths = newItems.map((x) => x.coverArtPath).toSet().toList();

  final grid = generateTilesOfSize(
    sizeDefinition,
    paths.length,
    DateTime.now().millisecondsSinceEpoch,
    gridSize.ceil(),
  );

  final images = await Future.wait(
    paths.map(
      (path) => loadAndResizeImage(
        path,
        gridSize.ceil() * maxRandomGridConfigSize,
      ),
    ),
  );

  final painter = CoverWallBackgroundPainter(
    grid: grid,
    gridSize: gridSize,
    gap: gap,
    images: images,
  );

  final size = ui.Size(1920, 1080);
  painter.paint(canvas, size);

  return recorder
      .endRecording()
      .toImage(size.width.floor(), size.height.floor());
}
