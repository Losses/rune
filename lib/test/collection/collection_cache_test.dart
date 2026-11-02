import 'package:flutter_test/flutter_test.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../bindings/bindings.dart';

void main() {
  test(
    'invalidating the cache advances its generation and removes cached data',
    () {
      final cache = CollectionCache();
      cache.clearAll();
      final before = cache.generation;
      cache.cacheSummary(CollectionType.album, []);
      expect(cache.getSummary(CollectionType.album), isNotNull);
      cache.clearAll();
      expect(cache.generation, greaterThan(before));
      expect(cache.getSummary(CollectionType.album), isNull);
    },
  );
}
