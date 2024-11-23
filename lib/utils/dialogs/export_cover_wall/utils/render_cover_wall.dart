import 'dart:ui' as ui;

import 'package:flutter/services.dart';
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

Future<ui.Image> loadImageFromAsset(String assetPath) async {
  final ByteData data = await rootBundle.load(assetPath);
  final Uint8List bytes = data.buffer.asUint8List();
  final codec = await ui.instantiateImageCodec(bytes);
  final frame = await codec.getNextFrame();
  return frame.image;
}

Future<ui.Image> renderCoverWall(
  CollectionType type,
  int id,
  Size size,
  Color background,
  bool frame,
  Color watermarkColor,
) async {
  final sizeDefinition =
      BoxConstraints(maxWidth: size.width, maxHeight: size.height);
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

  final backgroundPaint = Paint()
    ..color = background
    ..style = PaintingStyle.fill;

  canvas.drawRect(
    Rect.fromLTWH(0, 0, size.width, size.height),
    backgroundPaint,
  );

  final painter = CoverWallBackgroundPainter(
    grid: grid,
    gridSize: gridSize,
    gap: gap,
    images: images,
  );

  painter.paint(canvas, size);

  if (frame) {
    const strokeSize = 16.0;
    const bottomSize = 100.0;

    final borderPaint = Paint()
      ..color = background
      ..style = PaintingStyle.stroke;

    borderPaint.strokeWidth = strokeSize;

    final path = Path();

    path.moveTo(0, strokeSize / 2);
    path.lineTo(size.width, strokeSize / 2);
    canvas.drawPath(path, borderPaint);

    path.reset();
    path.moveTo(size.width - strokeSize / 2, strokeSize);
    path.lineTo(size.width - strokeSize / 2, size.height - bottomSize / 2);
    canvas.drawPath(path, borderPaint);

    path.reset();
    path.moveTo(strokeSize / 2, strokeSize);
    path.lineTo(strokeSize / 2, size.height - bottomSize / 2);
    canvas.drawPath(path, borderPaint);

    borderPaint.strokeWidth = bottomSize;

    path.reset();
    path.moveTo(0, size.height - bottomSize / 2);
    path.lineTo(size.width, size.height - bottomSize / 2);
    canvas.drawPath(path, borderPaint);

    final watermarkPaint = Paint()
      ..colorFilter = ColorFilter.mode(
        watermarkColor,
        BlendMode.srcATop,
      );

    final position = Offset(
      0,
      (size.height - 100),
    );

    final watermark = await loadImageFromAsset('assets/watermark.png');

    canvas.drawImage(watermark, position, watermarkPaint);
  }

  return recorder
      .endRecording()
      .toImage(size.width.floor(), size.height.floor());
}
