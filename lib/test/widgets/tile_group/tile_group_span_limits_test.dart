import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rune/widgets/start_screen/start_group_implementation.dart';
import 'package:rune/widgets/tile_group/tile_layout.dart';

// The spans a caller returns are not trusted to fit the group: they are
// clamped to what the block can show.

const _cell = 120.0;
const _gap = 4.0;
// Four columns for the square calculator, three rows for the default one.
const _squareHeight = 528.0;
const _defaultHeight = 396.0;

StartGroupImplementation<int> _square(
  int count,
  TileSpan Function(int item) spanOf,
) {
  return StartGroupImplementation<int>.square(
    cellSize: _cell,
    gapSize: _gap,
    items: List<int>.generate(count, (i) => i),
    groupIndex: 0,
    constraints: const BoxConstraints.tightFor(height: _squareHeight),
    itemBuilder: (context, item) => const SizedBox.shrink(),
    direction: Axis.horizontal,
    tileSpanBuilder: spanOf,
  );
}

void main() {
  test('A tile keeps the rows the final block gives it', () {
    // Counting the tiles as cells would give one row at first, and the final
    // block has four. The first tile must not stay at the smaller size.
    final m = _square(
      8,
      (i) => i == 0 ? const TileSpan(2, 2) : const TileSpan(1, 1),
    ).measure();

    expect(m.spans.first.rows, 2);
    expect(m.spans.first.columns, 2);
  });

  test('A square group is never wider than its block', () {
    final m = _square(
      8,
      (i) => i == 0 ? const TileSpan(9, 1) : const TileSpan(1, 1),
    ).measure();

    expect(m.layout.columns, lessThanOrEqualTo(4));
    expect(m.size.width, 4 * (_cell + _gap) - _gap);
  });

  test('Spans below one cell are shown as one cell', () {
    final group = StartGroupImplementation<int>(
      cellSize: _cell,
      gapSize: _gap,
      items: List<int>.generate(6, (i) => i),
      groupIndex: 0,
      constraints: const BoxConstraints.tightFor(height: _defaultHeight),
      itemBuilder: (context, item) => const SizedBox.shrink(),
      direction: Axis.horizontal,
      tileSpanBuilder: (i) =>
          i == 0 ? const TileSpan(-2, -9) : const TileSpan(1, 1),
    );

    final m = group.measure();

    expect(m.spans.length, 6);
    expect(m.spans.first.columns, 1);
    expect(m.spans.first.rows, 1);
  });
}
