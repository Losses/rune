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

// The titles of pages 0 to count - 1 as the fake loader names them.
List<String> _pages(String prefix, int count) => [
  for (var page = 0; page < count; page++)
    ..._titles(_groups('$prefix$page-', 3)),
];

void main() {
  late StreamController<void> contentChanged;
  late Map<int, Completer<(_Groups, bool)>> pending;
  late String prefix;
  late int lastPage;

  CollectionDataProvider makeProvider() => CollectionDataProvider(
    collectionType: CollectionType.album,
    contentChanged: contentChanged.stream,
    loadSummary: (_) async => [],
    loadPage: (type, size, cursor) {
      final completer = pending[cursor];
      if (completer != null) return completer.future;
      return Future.value((_groups('$prefix$cursor-', 3), cursor >= lastPage));
    },
  );

  setUp(() {
    CollectionCache().clearAll();
    contentChanged = StreamController<void>.broadcast(sync: true);
    pending = {};
    prefix = 'old';
    lastPage = 1;
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

  test(
    'after a failed reload the next fetch starts again from page zero',
    () async {
      final provider = makeProvider();
      await provider.fetchData();
      await provider.fetchData();
      expect(_titles(provider.items), _pages('old', 2));

      prefix = 'new';
      pending = {0: Completer<(_Groups, bool)>()};
      final reload = provider.reloadData();
      pending[0]!.completeError(StateError('offline'));
      await reload;
      expect(provider.pageError, isStateError);
      expect(_titles(provider.items), _pages('old', 2));

      pending = {};
      await provider.fetchData();
      expect(_titles(provider.items), _pages('new', 2));
      expect(provider.cursor, 2);
      provider.dispose();
    },
  );

  test('a page asked for during a reload is fetched after it', () async {
    lastPage = 3;
    final provider = makeProvider();
    await provider.fetchData();
    await provider.fetchData();
    expect(provider.isLastPage, isFalse);

    prefix = 'new';
    pending = {
      0: Completer<(_Groups, bool)>(),
      1: Completer<(_Groups, bool)>(),
    };
    contentChanged.add(null);
    // The list asks once at its end; asking twice still fetches one page.
    final fetch = provider.fetchData();
    final again = provider.fetchData();

    // The reload ends on pages that are not the last.
    pending[0]!.complete((_groups('new0-', 3), false));
    pending[1]!.complete((_groups('new1-', 3), false));
    await Future.wait([fetch, again]);
    expect(_titles(provider.items), _pages('new', 3));
    expect(provider.cursor, 3);
    expect(provider.isLastPage, isFalse);
    expect(provider.isLoading, isFalse);

    // Paging goes on from there.
    await provider.fetchData();
    expect(_titles(provider.items), _pages('new', 4));
    expect(provider.isLastPage, isTrue);
    provider.dispose();
  });

  test(
    'a page asked for during a reload that then fails is not appended',
    () async {
      lastPage = 2;
      final provider = makeProvider();
      await provider.fetchData();
      await provider.fetchData();

      prefix = 'new';
      pending = {2: Completer<(_Groups, bool)>()};
      final reload = provider.reloadData();
      final fetch = provider.fetchData();
      await Future<void>.delayed(Duration.zero);
      pending[2]!.completeError(StateError('offline'));
      // The failure is the provider's page error, as a failed page is.
      await Future.wait([reload, fetch]);
      expect(provider.pageError, isStateError);
      expect(_titles(provider.items), _pages('old', 2));
      expect(provider.cursor, 2);
      expect(provider.isLoading, isFalse);

      pending = {};
      await provider.fetchData();
      expect(_titles(provider.items), _pages('new', 3));
      expect(provider.cursor, 3);
      provider.dispose();
    },
  );

  test(
    'an append that a failed reload cuts short leaves the list ready to load',
    () async {
      lastPage = 2;
      final provider = makeProvider();
      await provider.fetchData();

      final appendPage = Completer<(_Groups, bool)>();
      pending = {1: appendPage};
      final append = provider.fetchData();
      expect(provider.isLoading, isTrue);
      expect(provider.cursor, 1);

      prefix = 'new';
      pending = {0: Completer<(_Groups, bool)>()};
      final reload = provider.reloadData();
      pending[0]!.completeError(StateError('offline'));
      await reload;
      expect(provider.pageError, isStateError);
      expect(provider.isLoading, isFalse);
      expect(provider.cursor, 1);
      expect(_titles(provider.items), _pages('old', 1));

      // The cut-short append's page arrives late and is dropped.
      appendPage.complete((_groups('old1-', 3), false));
      await append;
      expect(_titles(provider.items), _pages('old', 1));

      pending = {};
      await provider.fetchData();
      expect(_titles(provider.items), _pages('new', 2));
      expect(provider.cursor, 2);
      expect(provider.isLoading, isFalse);
      provider.dispose();
    },
  );

  test('a reload fetches the page of an append it cuts short', () async {
    lastPage = 2;
    final provider = makeProvider();
    await provider.fetchData();

    final appendPage = Completer<(_Groups, bool)>();
    pending = {1: appendPage};
    final append = provider.fetchData();

    prefix = 'new';
    pending = {};
    await provider.reloadData();
    expect(_titles(provider.items), _pages('new', 2));
    expect(provider.cursor, 2);
    expect(provider.isLoading, isFalse);

    appendPage.complete((_groups('old1-', 3), false));
    await append;
    expect(_titles(provider.items), _pages('new', 2));
    provider.dispose();
  });

  test('a content change whose reload fails keeps the items and reports the '
      'page error without throwing', () async {
    final provider = makeProvider();
    await provider.fetchData();
    await provider.fetchData();

    prefix = 'new';
    pending = {0: Completer<(_Groups, bool)>()};
    contentChanged.add(null);
    pending[0]!.completeError(StateError('offline'));
    // An error nobody handled would fail this test.
    await pumpEventQueue();

    expect(provider.pageError, isStateError);
    expect(_titles(provider.items), _pages('old', 2));
    expect(provider.isLoading, isFalse);
    provider.dispose();
  });

  test('a burst of content changes makes at most two reload passes', () async {
    var passes = 0;
    final gate = Completer<(_Groups, bool)>();
    final provider = CollectionDataProvider(
      collectionType: CollectionType.album,
      contentChanged: contentChanged.stream,
      loadSummary: (_) async => [],
      loadPage: (type, size, cursor) {
        passes++;
        return passes == 1
            ? gate.future
            : Future.value((_groups('x', 3), true));
      },
    );
    for (var i = 0; i < 5; i++) {
      contentChanged.add(null);
    }
    await Future<void>.delayed(Duration.zero);
    gate.complete((_groups('x', 3), true));
    await Future<void>.delayed(const Duration(milliseconds: 20));
    expect(passes, 2);
    provider.dispose();
  });
}
