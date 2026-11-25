import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../utils/api/set_download_rule.dart';

void main() {
  test('offline selections use the existing collection query operators', () {
    for (final entry in [
      (CollectionType.track, 'lib::track'),
      (CollectionType.artist, 'lib::artist'),
      (CollectionType.album, 'lib::album'),
      (CollectionType.playlist, 'lib::playlist'),
    ]) {
      final (queries, mixId) = offlineSelectionFor(entry.$1, 42);
      expect(mixId, isNull);
      expect(queries, hasLength(1));
      expect(queries.single.operator, entry.$2);
      expect(queries.single.parameter, '42');
    }
  });

  test('saved mixes retain their identity for reevaluation', () {
    final (queries, mixId) = offlineSelectionFor(CollectionType.mix, 12);
    expect(queries, isEmpty);
    expect(mixId, 12);
  });
}
