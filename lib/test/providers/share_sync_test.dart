import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_manager.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late List<String> requested;
  late Map<String, Completer<ShareSyncFailure?>> pending;
  late LibraryManagerProvider manager;
  late int notifications;

  setUp(() {
    requested = [];
    pending = {};
    notifications = 0;
    manager = LibraryManagerProvider(
      sendScanRequest: (_) {},
      libraryTaskStates: const Stream.empty(),
      requestFailures: const Stream.empty(),
      syncShareLibrary: (libraryId) {
        requested.add(libraryId);
        return (pending[libraryId] = Completer<ShareSyncFailure?>()).future;
      },
    )..addListener(() => notifications++);
  });
  tearDown(() => manager.dispose());

  test('a share sync is tracked from its start to its end', () async {
    expect(manager.isShareSyncing('a'), isFalse);

    final sync = manager.syncShareLibrary('a');
    expect(manager.isShareSyncing('a'), isTrue);
    expect(manager.isShareSyncing('b'), isFalse);
    expect(notifications, 1);

    pending['a']!.complete(ShareSyncFailure.readOnlyShare);
    expect(await sync, ShareSyncFailure.readOnlyShare);
    expect(manager.isShareSyncing('a'), isFalse);
    expect(notifications, 2);
  });

  test('a second call for a syncing library joins the running sync', () async {
    final first = manager.syncShareLibrary('a');
    final second = manager.syncShareLibrary('a');
    final other = manager.syncShareLibrary('b');

    expect(second, same(first));
    expect(other, isNot(same(first)));
    expect(requested, ['a', 'b']);

    pending['a']!.complete(null);
    expect(await first, isNull);
    expect(manager.isShareSyncing('a'), isFalse);
    expect(manager.isShareSyncing('b'), isTrue);

    final again = manager.syncShareLibrary('a');
    expect(again, isNot(same(first)));
    expect(requested, ['a', 'b', 'a']);
    pending['a']!.complete(null);
    pending['b']!.complete(null);
    await Future.wait([again, other]);
  });

  test(
    'a refused share sync ends the tracking and reports the error',
    () async {
      final sync = manager.syncShareLibrary('a');
      final error = StateError('not a share library');

      pending['a']!.completeError(error);

      await expectLater(sync, throwsA(same(error)));
      expect(manager.isShareSyncing('a'), isFalse);
      expect(notifications, 2);
    },
  );

  test(
    'a share sync that ends after the provider is gone is ignored',
    () async {
      final unlistened = LibraryManagerProvider(
        sendScanRequest: (_) {},
        libraryTaskStates: const Stream.empty(),
        requestFailures: const Stream.empty(),
        syncShareLibrary: (libraryId) =>
            (pending[libraryId] = Completer<ShareSyncFailure?>()).future,
      );
      final sync = unlistened.syncShareLibrary('a');
      unlistened.dispose();

      pending['a']!.complete(null);

      expect(await sync, isNull);
    },
  );
}
