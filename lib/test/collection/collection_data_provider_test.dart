import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../../bindings/bindings.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';

typedef _Groups = List<Group<InternalCollection>>;

_Groups _groups(String prefix, int count) => [
  for (var i = 0; i < count; i++)
    Group<InternalCollection>(groupTitle: '$prefix$i', items: []),
];

List<String> _titles(_Groups groups) => [for (final g in groups) g.groupTitle];

void main() {
  late StreamController<void> contentChanged;
  late Map<int, Completer<(_Groups, bool)>> pending;
  late String prefix;

  CollectionDataProvider makeProvider() => CollectionDataProvider(
    collectionType: CollectionType.album,
    contentChanged: contentChanged.stream,
    loadSummary: (_) async => [],
    loadPage: (type, size, cursor) {
      final completer = pending[cursor];
      if (completer != null) return completer.future;
      return Future.value((_groups('$prefix$cursor-', 3), cursor >= 1));
    },
  );

  setUp(() {
    CollectionCache().clearAll();
    contentChanged = StreamController<void>.broadcast(sync: true);
    pending = {};
    prefix = 'old';
  });

  tearDown(() => contentChanged.close());

  test(
    'a content change keeps the old items until fresh pages arrive',
    () async {
      final provider = makeProvider();
      await provider.fetchData();
      await provider.fetchData();
      final before = _titles(provider.items);
      expect(before.length, 6);
      expect(provider.cursor, 2);

      prefix = 'new';
      pending = {
        0: Completer<(_Groups, bool)>(),
        1: Completer<(_Groups, bool)>(),
      };
      contentChanged.add(null);
      await Future<void>.delayed(Duration.zero);
      expect(_titles(provider.items), before);

      pending[0]!.complete((_groups('new0-', 3), false));
      await Future<void>.delayed(Duration.zero);
      expect(_titles(provider.items), before);

      pending[1]!.complete((_groups('new1-', 3), true));
      await Future<void>.delayed(Duration.zero);
      expect(_titles(provider.items), [
        ..._titles(_groups('new0-', 3)),
        ..._titles(_groups('new1-', 3)),
      ]);
      expect(provider.cursor, 2);
      expect(provider.isLastPage, isTrue);
      provider.dispose();
    },
  );

  test(
    'fetchData during a reload waits for it and adds no duplicates',
    () async {
      final provider = makeProvider();
      await provider.fetchData();
      await provider.fetchData();

      prefix = 'new';
      pending = {
        0: Completer<(_Groups, bool)>(),
        1: Completer<(_Groups, bool)>(),
      };
      contentChanged.add(null);
      await Future<void>.delayed(Duration.zero);

      var done = false;
      final fetch = provider.fetchData().then((_) => done = true);
      await Future<void>.delayed(Duration.zero);
      expect(done, isFalse);

      pending[0]!.complete((_groups('new0-', 3), false));
      pending[1]!.complete((_groups('new1-', 3), true));
      await fetch;
      expect(_titles(provider.items).length, 6);
      expect(provider.cursor, 2);
      provider.dispose();
    },
  );
}
