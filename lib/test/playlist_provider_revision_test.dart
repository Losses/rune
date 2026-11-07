import 'dart:async';

import 'package:flutter_test/flutter_test.dart';

import '../bindings/bindings.dart';
import '../providers/playlist.dart';

void main() {
  test(
    'sends the latest queue revision and takes the refreshed queue after a rejected edit',
    () async {
      final updates = StreamController<PlaylistUpdate>();
      final failures = StreamController<RequestFailedResponse>();
      final sent = <MovePlaylistItemRequest>[];
      final removed = <RemoveRequest>[];
      final switched = <SwitchRequest>[];
      final provider = PlaylistProvider(
        playlistUpdates: updates.stream,
        requestFailures: failures.stream,
        sendMoveRequest: sent.add,
        sendRemoveRequest: removed.add,
        sendSwitchRequest: switched.add,
      );
      addTearDown(() async {
        provider.dispose();
        await updates.close();
        await failures.close();
      });
      const item = PlaylistItem(
        item: PlayingItemRequest(),
        artist: '',
        album: '',
        title: '',
        duration: 0,
      );
      const secondItem = PlaylistItem(
        item: PlayingItemRequest(),
        artist: '',
        album: '',
        title: 'second',
        duration: 0,
      );

      updates.add(
        PlaylistUpdate(
          items: [item, secondItem],
          sessionId: 'session',
          revision: Uint64.fromBigInt(BigInt.from(12)),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      provider.reorderItems(0, 2);

      expect(sent, hasLength(1));
      expect(sent.single.expectedRevision, Uint64.fromBigInt(BigInt.from(12)));
      provider.removeItem(0);
      expect(
        removed.single.expectedRevision,
        Uint64.fromBigInt(BigInt.from(12)),
      );
      provider.playItem(1);
      expect(switched.single.index, 1);
      expect(
        switched.single.expectedRevision,
        Uint64.fromBigInt(BigInt.from(12)),
      );
      final conflictFeedback = provider.failureFeedback.first;
      failures.add(
        const RequestFailedResponse(
          request: 'SwitchRequest',
          kind: RequestErrorKind.conflict,
          detail: 'stale queue revision',
        ),
      );
      final failure = await conflictFeedback;
      expect(failure.request, 'SwitchRequest');
      expect(failure.kind, RequestErrorKind.conflict);
      expect(provider.items.map((entry) => entry.entry.title), ['second', '']);
      updates.add(
        PlaylistUpdate(
          items: [item, secondItem],
          sessionId: 'session',
          revision: Uint64.fromBigInt(BigInt.from(13)),
        ),
      );
      await Future<void>.delayed(Duration.zero);
      // The renderer's queue replaces the optimistic reorder, and the next edit
      // is made against the refreshed revision.
      expect(provider.items.map((entry) => entry.entry.title), ['', 'second']);
      provider.reorderItems(1, 0);
      expect(sent.last.expectedRevision, Uint64.fromBigInt(BigInt.from(13)));
    },
  );
}
