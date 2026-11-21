// winjs_fixtures.json holds 300 layouts recorded from WinJS. Source: ax-design/tile-group
// 0.2.4, test/oracle/winjs-fixtures.json (MIT, copyright ax-design, license text in
// tile_layout.dart).
import 'dart:convert';
import 'dart:io';

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
      final actual = layoutTiles(spans, rows: f['rows'] as int).placements;
      expect(actual, expected, reason: 'fixture $i');
    }
  });
}
