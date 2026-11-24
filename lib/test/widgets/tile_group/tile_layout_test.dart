// winjs_fixtures.json holds 300 layouts recorded from WinJS (MIT, Copyright (c) Microsoft
// Corporation). Source: ax-design/tile-group
// 0.2.4, test/oracle/winjs-fixtures.json (MIT, copyright ax-design, license text in
// tile_layout.dart).
import 'dart:convert';
import 'dart:io';
import 'dart:math';

import 'package:flutter_test/flutter_test.dart';

import '../../../widgets/tile_group/tile_layout.dart';

void main() {
  test('matches the WinJS cell-spanning layout', () {
    final fixtures =
        jsonDecode(
              File(
                'lib/test/widgets/tile_group/winjs_fixtures.json',
              ).readAsStringSync(),
            )
            as List<dynamic>;
    expect(fixtures, isNotEmpty);
    for (var i = 0; i < fixtures.length; i++) {
      final f = fixtures[i] as Map<String, dynamic>;
      final spans = [
        for (final it in f['items'] as List<dynamic>)
          TileSpan(
            (it as List<dynamic>)[0] as int,
            it[1] as int,
            newColumn: it[2] == 1,
          ),
      ];
      final expected = [
        for (final p in f['winjs'] as List<dynamic>)
          TilePlacement((p as List<dynamic>)[0] as int, p[1] as int),
      ];
      final rows = f['rows'] as int;
      final actual = layoutTiles(spans, rows: rows);
      expect(actual.placements, expected, reason: 'fixture $i');

      // Columns and rows are derived from the recorded placements: the
      // columns reached by the tiles, and the rows they cover within the
      // group. Spans count as at least 1, as in layoutTiles.
      var columns = 0;
      var usedRows = 0;
      for (var k = 0; k < spans.length; k++) {
        columns = max(columns, expected[k].column + max(1, spans[k].columns));
        usedRows = max(
          usedRows,
          min(rows, expected[k].row + max(1, spans[k].rows)),
        );
      }
      expect(actual.columns, columns, reason: 'columns of fixture $i');
      expect(actual.rows, usedRows, reason: 'rows of fixture $i');
    }
  });
}
