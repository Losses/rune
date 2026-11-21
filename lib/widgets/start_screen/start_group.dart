import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/tile_layout.dart';
import './normal_layout.dart';
import './stacked_layout.dart';

import 'start_group_item.dart';
import 'start_group_implementation.dart';

enum StartGroupGridLayoutVariation { initial, square }

enum StartGroupGroupLayoutVariation { normal, stacked }

const double defaultCellSize = 120;

class StartGroup<T> extends StatelessWidget {
  final List<T> items;
  final String groupTitle;
  final String? groupLink;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;
  final StartGroupGridLayoutVariation gridLayoutVariation;
  final StartGroupGroupLayoutVariation groupLayoutVariation;
  final BoxConstraints constraints;
  final Dimensions Function(double, double, double, List<T>)?
      dimensionCalculator;
  final Axis direction;

  /// Size in cells of the tile of an item. One cell when null.
  final TileSpan Function(T item)? tileSpanBuilder;

  /// Width in columns of the strips the tiles fill, see [layoutTiles]. When
  /// null, tiles fill the group row by row for [Axis.horizontal] and column
  /// by column for [Axis.vertical].
  final int? strip;

  /// Size of one cell.
  final double cellSize;

  final double gapSize;
  final VoidCallback? onTitleTap;

  const StartGroup({
    super.key,
    required this.groupIndex,
    required this.groupTitle,
    required this.items,
    required this.itemBuilder,
    required this.constraints,
    this.groupLink,
    this.gapSize = 4,
    this.onTitleTap,
    this.gridLayoutVariation = StartGroupGridLayoutVariation.initial,
    this.groupLayoutVariation = StartGroupGroupLayoutVariation.normal,
    this.dimensionCalculator,
    required this.direction,
    this.tileSpanBuilder,
    this.strip,
    this.cellSize = defaultCellSize,
  });

  /// The width the group takes in a horizontal list, padding included, as
  /// [build] lays it out. It lets a list scroll to a group it has not built yet.
  double widthIn(BuildContext context) {
    final childWidth = _buildStartGroupItems().measure().size.width;
    switch (groupLayoutVariation) {
      case StartGroupGroupLayoutVariation.stacked:
        return StartGroupStackedLayout.widthFor(
          context,
          groupTitle,
          childWidth,
        );
      case StartGroupGroupLayoutVariation.normal:
        return StartGroupNormalLayout.widthFor(
          context,
          groupTitle,
          childWidth,
        );
    }
  }

  @override
  Widget build(BuildContext context) {
    return FocusTraversalGroup(
      child: _buildStartGroupLayout(
        context,
        _buildStartGroupItems(),
      ),
    );
  }

  Widget _buildStartGroupLayout(BuildContext context, Widget child) {
    switch (groupLayoutVariation) {
      case StartGroupGroupLayoutVariation.stacked:
        return StartGroupStackedLayout(
          groupTitle: groupTitle,
          onTitleTap: onTitleTap,
          child: child,
        );
      case StartGroupGroupLayoutVariation.normal:
        return StartGroupNormalLayout(
          groupTitle: groupTitle,
          onTitleTap: onTitleTap,
          child: child,
        );
    }
  }

  StartGroupImplementation<T> _buildStartGroupItems() {
    switch (gridLayoutVariation) {
      case StartGroupGridLayoutVariation.square:
        return StartGroupImplementation<T>.square(
          cellSize: cellSize,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          constraints: constraints,
          itemBuilder: itemBuilder,
          direction: direction,
          tileSpanBuilder: tileSpanBuilder,
          strip: strip,
        );
      case StartGroupGridLayoutVariation.initial:
        return StartGroupImplementation<T>(
          cellSize: cellSize,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          constraints: constraints,
          itemBuilder: itemBuilder,
          dimensionCalculator: dimensionCalculator,
          direction: direction,
          tileSpanBuilder: tileSpanBuilder,
          strip: strip,
        );
    }
  }
}
