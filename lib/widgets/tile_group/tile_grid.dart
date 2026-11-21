import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import 'tile_layout.dart';

/// Places tiles at the cells given by a [TileLayout].
///
/// Children keep their State when the layout changes as long as their order
/// stays the same; key them when the order can change.
class TileGrid extends StatelessWidget {
  const TileGrid({
    super.key,
    required this.layout,
    required this.spans,
    required this.cellSize,
    required this.gapSize,
    required this.size,
    required this.children,
  });

  /// Where each tile goes.
  final TileLayout layout;

  /// Size in cells of each tile.
  final List<TileSpan> spans;
  final double cellSize;
  final double gapSize;

  /// The box of the grid itself.
  final Size size;

  /// One child per span, in the same order.
  final List<Widget> children;

  /// Length of [cells] cells with a gap between each two of them.
  static double extent(int cells, double cellSize, double gapSize) =>
      max(0.0, cells * (cellSize + gapSize) - gapSize);

  /// The box of a tile, relative to the top left of the grid.
  static Rect rectOf(
    TilePlacement placement,
    TileSpan span,
    double cellSize,
    double gapSize,
  ) {
    return Rect.fromLTWH(
      placement.column * (cellSize + gapSize),
      placement.row * (cellSize + gapSize),
      extent(max(1, span.columns), cellSize, gapSize),
      extent(max(1, span.rows), cellSize, gapSize),
    );
  }

  /// The size of the cells in use.
  static Size packedSize(TileLayout layout, double cellSize, double gapSize) {
    return Size(
      extent(layout.columns, cellSize, gapSize),
      extent(layout.rows, cellSize, gapSize),
    );
  }

  @override
  Widget build(BuildContext context) {
    assert(
      children.length == spans.length &&
          spans.length == layout.placements.length,
      'children, spans and layout.placements must have the same length',
    );
    // Read the direction once: a PositionedDirectional per tile would add a
    // widget and a dependency on Directionality for every tile.
    final textDirection = Directionality.of(context);
    return SizedBox.fromSize(
      size: size,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          for (var i = 0; i < children.length; i++)
            _positioned(
              rectOf(layout.placements[i], spans[i], cellSize, gapSize),
              textDirection,
              children[i],
            ),
        ],
      ),
    );
  }

  Widget _positioned(Rect r, TextDirection textDirection, Widget child) {
    return Positioned.directional(
      textDirection: textDirection,
      start: r.left,
      top: r.top,
      width: r.width,
      height: r.height,
      child: child,
    );
  }
}
