import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/status.dart';
import '../../widgets/playback_controller/queue_button.dart';

void main() {
  testWidgets('queue flyout row menu removes the selected item', (
    tester,
  ) async {
    final updates = StreamController<PlaylistUpdate>();
    final failures = StreamController<RequestFailedResponse>();
    final removed = <RemoveRequest>[];
    final playlist = PlaylistProvider(
      playlistUpdates: updates.stream,
      requestFailures: failures.stream,
      sendRemoveRequest: removed.add,
    );
    addTearDown(() async {
      playlist.dispose();
      await updates.close();
      await failures.close();
    });
    updates.add(
      PlaylistUpdate(
        items: [
          const PlaylistItem(
            item: PlayingItemRefIndependentFile(path: ''),
            artist: 'Artist',
            album: 'Album',
            title: 'Queue track',
            duration: 60,
          ),
        ],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(3)),
      ),
    );

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<PlaylistProvider>.value(value: playlist),
          ChangeNotifierProvider(create: (_) => RequestCapabilitiesProvider()),
          ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
        ],
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(
            content: Center(child: QueueButton(shadows: null)),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.byType(QueueButton));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Queue track'), buttons: kSecondaryMouseButton);
    await tester.pumpAndSettle();

    expect(find.text('Remove from Playlist'), findsOneWidget);
    await tester.tap(find.text('Remove from Playlist'));
    await tester.pumpAndSettle();

    expect(removed, hasLength(1));
    expect(removed.single.index, 0);
    expect(removed.single.expectedRevision, Uint64.fromBigInt(BigInt.from(3)));
  });
}
