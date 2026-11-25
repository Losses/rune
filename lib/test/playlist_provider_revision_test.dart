import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../bindings/bindings.dart';
import '../l10n/app_localizations.dart';
import '../providers/playlist.dart';
import '../providers/request_capabilities.dart';
import '../providers/status.dart';
import '../widgets/playback_controller/queue.dart';

const _item = PlaylistItem(
  item: PlayingItemRefIndependentFile(path: ''),
  artist: '',
  album: '',
  title: '',
  duration: 0,
);
const _secondItem = PlaylistItem(
  item: PlayingItemRefIndependentFile(path: ''),
  artist: '',
  album: '',
  title: 'second',
  duration: 0,
);

void main() {
  test(
    'sends the latest queue revision and takes the refreshed queue after a rejected edit',
    () async {
      final updates = StreamController<PlaylistUpdate>(sync: true);
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
      updates.add(
        PlaylistUpdate(
          items: [_item, _secondItem],
          sessionId: 'session',
          revision: Uint64.fromBigInt(BigInt.from(12)),
        ),
      );
      provider.reorderItems(0, 2);

      expect(sent, hasLength(1));
      expect(sent.single.expectedRevision, Uint64.fromBigInt(BigInt.from(12)));
      provider.removeItem(0);
      expect(
        removed.single.expectedRevision,
        Uint64.fromBigInt(BigInt.from(12)),
      );
      expect(removed.single.callId, isNotNull);
      provider.playItem(1);
      expect(switched.single.index, 1);
      expect(switched.single.callId, isNotNull);
      expect(
        switched.single.expectedRevision,
        Uint64.fromBigInt(BigInt.from(12)),
      );
      final conflictFeedback = provider.failureFeedback.first;
      failures.add(
        RequestFailedResponse(
          request: 'diagnostic name is ignored',
          kind: RequestErrorKind.conflict,
          detail: 'stale queue revision',
          callId: Uint64.fromBigInt(BigInt.zero),
        ),
      );
      failures.add(
        RequestFailedResponse(
          request: 'diagnostic name is ignored',
          kind: RequestErrorKind.conflict,
          detail: 'stale queue revision',
          callId: switched.single.callId,
        ),
      );
      final failure = await conflictFeedback;
      expect(failure.callId, switched.single.callId?.toInt());
      expect(failure.kind, RequestErrorKind.conflict);
      expect(provider.items.map((entry) => entry.entry.title), ['second', '']);
      final moveConflictFeedback = provider.failureFeedback.first;
      failures.add(
        RequestFailedResponse(
          request: 'diagnostic name is ignored',
          kind: RequestErrorKind.conflict,
          detail: 'stale queue revision',
          callId: sent.single.callId,
        ),
      );
      expect((await moveConflictFeedback).callId, sent.single.callId?.toInt());
      expect(provider.items.map((entry) => entry.entry.title), ['', 'second']);
      updates.add(
        PlaylistUpdate(
          items: [_item, _secondItem],
          sessionId: 'session',
          revision: Uint64.fromBigInt(BigInt.from(14)),
        ),
      );
      // The renderer's queue replaces the optimistic reorder, and the next edit
      // is made against the refreshed revision.
      expect(provider.items.map((entry) => entry.entry.title), ['', 'second']);
      provider.reorderItems(1, 0);
      expect(sent.last.expectedRevision, Uint64.fromBigInt(BigInt.from(14)));
    },
  );

  test('remove conflict survives a playlist update', () async {
    final updates = StreamController<PlaylistUpdate>(sync: true);
    final failures = StreamController<RequestFailedResponse>.broadcast(
      sync: true,
    );
    final removed = <RemoveRequest>[];
    final provider = PlaylistProvider(
      playlistUpdates: updates.stream,
      requestFailures: failures.stream,
      sendRemoveRequest: removed.add,
    );
    addTearDown(() async {
      provider.dispose();
      await updates.close();
      await failures.close();
    });
    updates.add(
      PlaylistUpdate(
        items: [_item, _secondItem],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(12)),
      ),
    );
    provider.removeItem(0);
    final feedback = provider.failureFeedback.first;
    updates.add(
      PlaylistUpdate(
        items: [_secondItem],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(13)),
      ),
    );
    failures.add(
      RequestFailedResponse(
        request: 'diagnostic name is ignored',
        kind: RequestErrorKind.conflict,
        detail: 'stale queue revision',
        callId: removed.single.callId,
      ),
    );
    expect(
      (await feedback.timeout(const Duration(seconds: 2))).kind,
      RequestErrorKind.conflict,
    );
  });

  testWidgets('queue shows remove conflict feedback after a playlist update', (
    tester,
  ) async {
    final updates = StreamController<PlaylistUpdate>(sync: true);
    final failures = StreamController<RequestFailedResponse>.broadcast(
      sync: true,
    );
    final removed = <RemoveRequest>[];
    final provider = PlaylistProvider(
      playlistUpdates: updates.stream,
      requestFailures: failures.stream,
      sendRemoveRequest: removed.add,
    );
    final status = PlaybackStatusProvider();
    final capabilities = RequestCapabilitiesProvider();
    addTearDown(() async {
      await tester.pumpWidget(const SizedBox());
      provider.dispose();
      status.dispose();
      capabilities.dispose();
      await updates.close();
      await failures.close();
    });
    updates.add(
      PlaylistUpdate(
        items: [_item, _secondItem],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(4)),
      ),
    );
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<PlaylistProvider>.value(value: provider),
          ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
          ChangeNotifierProvider<RequestCapabilitiesProvider>.value(
            value: capabilities,
          ),
        ],
        child: const FluentApp(
          locale: Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(content: Queue()),
        ),
      ),
    );
    provider.removeItem(0);
    updates.add(
      PlaylistUpdate(
        items: [_secondItem],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(5)),
      ),
    );
    failures.add(
      RequestFailedResponse(
        request: 'diagnostic name is ignored',
        kind: RequestErrorKind.conflict,
        detail: 'stale queue revision',
        callId: removed.single.callId,
      ),
    );
    await tester.pump();
    expect(find.text('The queue changed. Try again.'), findsOneWidget);
  });
}
