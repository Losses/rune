import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rinf/rinf.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/library_registry.dart';
import 'package:rune/screens/collection/utils/collection_data_provider.dart';
import 'package:rune/utils/api/create_playlist.dart';
import 'package:rune/utils/api/request_failure.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('a failed collection page can be retried without skipping groups', (
    tester,
  ) async {
    await tester.runAsync(() async {
      final root = await Directory.systemTemp.createTemp(
        'rune-collection-page-',
      );
      addTearDown(() => root.delete(recursive: true));
      final music = await Directory.fromUri(root.uri.resolve('music')).create();
      final config = await Directory.fromUri(
        root.uri.resolve('profile'),
      ).create();
      const groupsSignal = 'FetchCollectionGroupsResponse';
      final deliverGroups = assignRustSignal[groupsSignal]!;
      void Function(Uint8List, Uint8List)? nextReply;
      final signals = {...assignRustSignal};
      var groupRequests = 0;
      signals[groupsSignal] = (bytes, binary) {
        groupRequests++;
        final deliver = nextReply ?? deliverGroups;
        nextReply = null;
        deliver(bytes, binary);
      };
      await initializeRust(signals);
      addTearDown(finalizeRust);
      await awaitResponse(
        'InitializeNodeRequest',
        InitializeNodeResponse.rustSignalStream,
        responseCallId: (response) => response.message.callId?.toInt(),
        send: (id) => InitializeNodeRequest(
          configPath: config.path,
          callId: id,
        ).sendSignalToRust(),
      );
      final registry = LibraryRegistryProvider();
      addTearDown(registry.dispose);
      await registry.ready;
      final record = await registry.registerLocalFolder(
        music.path,
        'Pagination',
        LibraryInitializeMode.portable,
      );
      final id = record.record!.recordId;
      addTearDown(() => registry.remove(id));
      await registry.open(id);

      const groups = ['A', 'B', 'C', 'D', 'E', 'F', 'G'];
      for (final title in groups) {
        final change = LibraryContentChanged.rustSignalStream.first;
        await createPlaylist('Playlist $title', title);
        await change.timeout(const Duration(seconds: 10));
      }
      final data = CollectionDataProvider(
        collectionType: CollectionType.playlist,
      );
      addTearDown(data.dispose);
      var publications = 0;
      data.addListener(() => publications++);
      Iterable<String> titles() => data.items.map((group) => group.groupTitle);
      await data.fetchData();
      expect(titles(), groups.take(3));

      late List<String> failedGroups;
      nextReply = (bytes, binary) {
        final reply = FetchCollectionGroupsResponse.bincodeDeserialize(bytes);
        failedGroups = reply.groups.map((group) => group.groupTitle).toList();
        final failure = RequestFailedResponse(
          request: 'FetchCollectionGroupsRequest',
          callId: reply.callId,
          kind: RequestErrorKind.unavailable,
          detail: 'Collection page unavailable',
        );
        assignRustSignal['RequestFailedResponse']!(
          failure.bincodeSerialize(),
          binary,
        );
      };
      await data.fetchData();
      expect(
        data.pageError,
        isA<RequestFailure>().having(
          (error) => error.kind,
          'kind',
          RequestErrorKind.unavailable,
        ),
      );
      expect(titles(), groups.take(3));
      expect(data.isLoading, isFalse);
      expect(data.cursor, 1);
      expect(failedGroups, ['D', 'E', 'F']);
      await Future.wait([data.fetchData(), data.fetchData()]);
      expect(data.pageError, isNull);
      expect(titles(), groups.take(6));
      expect(data.cursor, 2);
      expect(groupRequests, 3); // The concurrent retries share one request.

      // Hold the final group's real reply until the refreshed first page is visible.
      final held = Completer<void Function()>();
      nextReply = (bytes, binary) =>
          held.complete(() => deliverGroups(bytes, binary));
      final oldPage = data.fetchData();
      final deliver = await held.future.timeout(const Duration(seconds: 10));
      await data.reloadData();
      final afterRefresh = publications;
      deliver();
      await oldPage;
      expect(publications, afterRefresh);
      expect(titles(), groups.take(3));
      expect(data.cursor, 1);
      expect(data.isLastPage, isFalse);
      expect(data.isLoading, isFalse);

      final leaving = CollectionDataProvider(
        collectionType: CollectionType.playlist,
      );
      final outstanding = leaving.fetchData();
      leaving.dispose();
      await outstanding;
      expect(leaving.items, isEmpty);
      expect(leaving.cursor, 0);
    });
  });
}
