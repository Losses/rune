import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../tile_group/tile_layout.dart';
import 'turntile_normal_layout.dart';
import 'turntile_group_items_list.dart';
import 'turntile_group_items_tile.dart';

enum TurntileGroupGridLayoutVariation { list, tile }

class TurntileGroup<T> extends StatelessWidget {
  final List<T> items;
  final String? groupTitle;
  final int groupIndex;
  final Widget Function(BuildContext, T) itemBuilder;
  final TurntileGroupGridLayoutVariation gridLayoutVariation;

  /// Size in cells of the tile of an item. One cell when null. The list
  /// variation keeps its own rows and ignores it.
  final TileSpan Function(T item)? tileSpanBuilder;

  /// Height in rows of the bands the tiles of the tile variation fill, see
  /// [layoutTiles]. 1 fills the group row by row. 2 fills bands of two rows
  /// column by column, so small tiles next to a medium one go top to bottom,
  /// as on the Windows 10 Start menu.
  final int strip;

  final double gapSize;
  final VoidCallback? onTitleTap;

  const TurntileGroup({
    super.key,
    required this.groupIndex,
    this.groupTitle,
    required this.items,
    required this.itemBuilder,
    this.gapSize = 4,
    this.onTitleTap,
    this.gridLayoutVariation = TurntileGroupGridLayoutVariation.list,
    this.tileSpanBuilder,
    this.strip = 1,
  });

  static const double _listCellSize = 40;
  static const double _tileCellSize = 88;

  /// The height the group takes in a vertical list [width] wide, padding
  /// included. It lets a list scroll to a group it has not built yet.
  double heightIn(BuildContext context, double width) {
    final double childHeight;
    switch (gridLayoutVariation) {
      case TurntileGroupGridLayoutVariation.list:
        childHeight = max(
          0.0,
          items.length * (_listCellSize + gapSize) - gapSize,
        );
      case TurntileGroupGridLayoutVariation.tile:
        childHeight = _tileItems()
            .measure(width - TurnTileGroupNormalLayout.padding.horizontal)
            .size
            .height;
    }
    return TurnTileGroupNormalLayout.heightFor(
      context,
      groupTitle,
      width,
      childHeight,
    );
  }

  @override
  Widget build(BuildContext context) {
    return _buildStartGroupLayout(context, _buildStartGroupItems());
  }

  Widget _buildStartGroupLayout(BuildContext context, Widget child) {
    return TurnTileGroupNormalLayout(
      groupTitle: groupTitle,
      onTitleTap: onTitleTap,
      child: child,
    );
  }

  Widget _buildStartGroupItems() {
    switch (gridLayoutVariation) {
      case TurntileGroupGridLayoutVariation.list:
        // A column of text links, not tiles: each row is as high as its text
        // and only the text is tappable, so it keeps its own rows instead of
        // the tile layout.
        return TurntileGroupItemsList<T>(
          cellSize: _listCellSize,
          gapSize: gapSize,
          items: items,
          groupIndex: groupIndex,
          itemBuilder: itemBuilder,
        );
      case TurntileGroupGridLayoutVariation.tile:
        return _tileItems();
    }
  }

  TurntileGroupItemsTile<T> _tileItems() {
    return TurntileGroupItemsTile<T>(
      cellSize: _tileCellSize,
      gapSize: gapSize,
      items: items,
      groupIndex: groupIndex,
      itemBuilder: itemBuilder,
      tileSpanBuilder: tileSpanBuilder,
      strip: strip,
    );
  }
}
