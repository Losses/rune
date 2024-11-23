import 'dart:math';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../screens/cover_wall/utils/string_to_double.dart';

import '../constants/max_random_grid_config_size.dart';
import '../constants/random_grid_config.dart';
import '../utils/calculate_cover_wall_size.dart';
import '../utils/random_grid_placement.dart';

List<List<RandomGridPlacement>> generateTilesOfSize(
  BoxConstraints constraints,
  int nImages,
  int seed,
  int size,
) {
  final gridSize = calculateCoverWallGridSize(constraints);
  final cols =
      (constraints.maxWidth / gridSize).ceil() + maxRandomGridConfigSize;
  final rows =
      (constraints.maxHeight / gridSize).ceil() + maxRandomGridConfigSize;

  final totalGrids = cols * rows;

  final List<bool> occupied = List.filled(totalGrids, false);
  final List<List<RandomGridPlacement>> placement =
      List.generate(nImages, (_) => [], growable: false);

  for (int i = 0; i < occupied.length; i += 1) {
    final int row = i ~/ cols;
    final int col = i % cols;

    final gridKey = '$row-$col';

    if (occupied[i]) continue;

    double randomValue1 = stringToDouble('$gridKey-$seed');
    double randomValue2 = stringToDouble('$gridKey-i-$seed');
    int coverIndex = (randomValue2 * (nImages - 1)).round();

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
    double randomValue2 = stringToDouble('$gridKey-i-$seed');
    int coverIndex = (randomValue2 * (nImages - 1)).round();

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
