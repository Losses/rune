// Ported from ax-design/tile-group's src/layout.ts (version 0.2.4).
//
// MIT License
//
// Copyright (c) 2026 ax-design
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
//
// The layout follows CellSpanningGroup of WinJS (https://github.com/winjs/winjs),
// MIT License, Copyright (c) Microsoft Corporation.

import 'package:flutter/painting.dart';

// Tile placement: the cell-spanning layout of the Windows 8 tile groups, after the
// CellSpanningGroup of WinJS (Microsoft's ListView for Windows 8 apps). No widgets.
//
// A group has a fixed number of rows of cells and grows to the right. Tiles keep their order:
// each one goes to the first free place, in scan order, that comes after the place of the tile
// before it. Places before it are not filled again, so a tile never moves in front of an
// earlier one.
//
// Scan order: the group is cut into strips of `strip` columns. Strips go left to right, inside
// a strip rows go top to bottom, inside a row columns go left to right. With strip 1 this is
// the column-major order of WinJS (top to bottom, then the next column). With strip 4 (two
// medium tiles of 2 x 2 cells) medium tiles fill a two-tile-wide column row by row.
//
// Axis.vertical is the same layout turned by a quarter, for groups of a fixed width
// that grow downward like the groups of the Windows 10 Start menu: `columns` is fixed, strips
// are bands of `strip` rows that go top to bottom, inside a band columns go left to right and
// inside a column rows go top to bottom. With strip 2 (one medium tile) medium tiles fill the
// group row by row.

/// How many cells a tile covers.
///
/// Sizes below 1 are treated as 1 by [layoutTiles] and by the start groups.
class TileSpan {
  /// A tile of [columns] by [rows] cells; [newColumn] makes it start a new strip.
  const TileSpan(this.columns, this.rows, {this.newColumn = false});

  /// Size in cells.
  final int columns;

  /// Size in cells, see [columns].
  final int rows;

  /// Start a new strip (column with strip 1) before this tile.
  final bool newColumn;
}

/// The top left cell of a placed tile.
class TilePlacement {
  /// A tile whose top left cell is at [column], [row], counted from 0.
  const TilePlacement(this.column, this.row);

  /// Column of the top left cell.
  final int column;

  /// Row of the top left cell.
  final int row;

  @override
  bool operator ==(Object other) =>
      other is TilePlacement && other.column == column && other.row == row;

  @override
  int get hashCode => Object.hash(column, row);

  @override
  String toString() => '($column, $row)';
}

/// The result of [layoutTiles].
class TileLayout {
  /// A layout of [placements] that uses [columns] by [rows] cells.
  const TileLayout({
    required this.placements,
    required this.columns,
    required this.rows,
  });

  /// One placement per span, in the order of the spans.
  final List<TilePlacement> placements;

  /// Columns in use, the width of the group in cells.
  final int columns;

  /// Rows in use, the height of the group in cells.
  final int rows;
}

/// Places [spans] in order.
///
/// [Axis.horizontal] (default): [rows] is fixed, the group grows to the right.
/// [Axis.vertical]: [columns] is fixed, the group grows downward. [strip] is
/// the size of a strip across the direction of growth, see above.
TileLayout layoutTiles(
  List<TileSpan> spans, {
  Axis orientation = Axis.horizontal,
  int rows = 1,
  int columns = 1,
  int strip = 1,
}) {
  if (orientation == Axis.vertical) {
    // The horizontal layout of the turned tiles, turned back.
    final turned = _layoutAcross(
      [
        for (final s in spans)
          TileSpan(s.rows, s.columns, newColumn: s.newColumn),
      ],
      columns,
      strip,
    );
    return TileLayout(
      placements: [
        for (final p in turned.placements) TilePlacement(p.row, p.column),
      ],
      columns: turned.rows,
      rows: turned.columns,
    );
  }
  return _layoutAcross(spans, rows, strip);
}

int _atLeastOne(int value) => value < 1 ? 1 : value;

TileLayout _layoutAcross(List<TileSpan> spans, int across, int stripSize) {
  final rows = _atLeastOne(across);
  final strip = _atLeastOne(stripSize);
  // occupied[column] holds the occupied rows of that column.
  final occupied = <int, Set<int>>{};
  var width = 0;

  bool free(int column, int row, TileSpan span) {
    if (row + span.rows > rows) return false;
    for (var c = column; c < column + span.columns; c++) {
      final col = occupied[c];
      if (col == null) continue;
      for (var r = row; r < row + span.rows; r++) {
        if (col.contains(r)) return false;
      }
    }
    return true;
  }

  void mark(int column, int row, TileSpan span) {
    // A tile taller than the group sticks out below; only its cells inside count.
    for (var c = column; c < column + span.columns; c++) {
      final col = occupied.putIfAbsent(c, () => <int>{});
      final end = row + span.rows < rows ? row + span.rows : rows;
      for (var r = row; r < end; r++) {
        col.add(r);
      }
    }
    final right = column + span.columns;
    if (right > width) width = right;
  }

  final placements = <TilePlacement>[];
  // Scan position of the previous tile: strip, row, column inside the strip.
  var cursorStrip = 0;
  var cursorRow = 0;
  var cursorColumn = 0;
  for (final raw in spans) {
    final span = TileSpan(
      _atLeastOne(raw.columns),
      _atLeastOne(raw.rows),
      newColumn: raw.newColumn,
    );
    // Strips that hold a cell, plus one past them: the scan never needs to go further.
    final lastStrip = (width + strip - 1) ~/ strip;
    TilePlacement? found;
    var s = cursorStrip;
    var row = cursorRow;
    var column = cursorColumn;
    if (span.newColumn && width > 0) {
      // The next strip that is empty at the top, as WinJS does for newColumn.
      for (s = cursorStrip + 1; s <= lastStrip && found == null; s++) {
        if (free(s * strip, 0, span)) found = TilePlacement(s * strip, 0);
      }
    } else {
      for (; s <= lastStrip && found == null; s++) {
        for (; row < rows && found == null; row++) {
          for (; column < strip && found == null; column++) {
            if (free(s * strip + column, row, span)) {
              found = TilePlacement(s * strip + column, row);
            }
          }
          column = 0;
        }
        row = 0;
      }
    }
    // Nothing fits (taller than the group, or every place taken): a new strip, at the top.
    if (found == null) {
      final next = cursorStrip + (width > 0 ? 1 : 0);
      found = TilePlacement((lastStrip > next ? lastStrip : next) * strip, 0);
    }
    mark(found.column, found.row, span);
    placements.add(found);
    cursorStrip = found.column ~/ strip;
    cursorRow = found.row;
    cursorColumn = found.column % strip;
  }
  // Height in use: the bottom of the lowest tile, within the group.
  var height = 0;
  for (var i = 0; i < placements.length; i++) {
    final bottom = placements[i].row + _atLeastOne(spans[i].rows);
    final inside = bottom < rows ? bottom : rows;
    if (inside > height) height = inside;
  }
  return TileLayout(placements: placements, columns: width, rows: height);
}
