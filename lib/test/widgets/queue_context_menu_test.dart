import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/status.dart';
import '../../utils/router/history.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../widgets/playback_controller/queue.dart';
import '../../widgets/playback_controller/queue_button.dart';
import '../../widgets/router/rune_with_navigation_bar_and_playback_controllor.dart';

enum _Dismissal { escape, outsideClick, ownerDisposal, coveredOwnerDisposal }

void main() {
  for (final dismissal in _Dismissal.values) {
    _testQueueDismissal(dismissal);
  }
}

void _testQueueDismissal(_Dismissal dismissal) {
  final name = 'remove a queue item, dismiss by ${dismissal.name}, and reopen';
  testWidgets(name, (tester) async {
    final updates = StreamController<PlaylistUpdate>();
    final failures = StreamController<RequestFailedResponse>();
    final removed = <RemoveRequest>[];
    final showOwner = ValueNotifier(true);
    addTearDown(showOwner.dispose);
    final history = $history.history.toList();
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
          navigatorKey: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
          navigatorObservers: [FlyoutRouteObserver()],
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: ScaffoldPage(
            content: ValueListenableBuilder<bool>(
              valueListenable: showOwner,
              builder: (context, visible, _) => Center(
                child: visible
                    ? Tooltip(
                        message: 'Queue',
                        child: controllerItems
                            .singleWhere((entry) => entry.id == 'playlist')
                            .controllerButtonBuilder(context, null),
                      )
                    : const SizedBox(),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tap(find.byType(QueueButton));
    await tester.pumpAndSettle();
    if (dismissal == _Dismissal.escape) {
      final owner = tester.state(find.byType(QueueButton));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: const Offset(4, 4));
      addTearDown(mouse.removePointer);
      await tester.pumpAndSettle();
      expect(
        tester.state(find.byType(QueueButton)),
        same(owner),
        reason: 'Connecting a mouse must preserve the open queue',
      );
    }
    await tester.tap(find.text('Queue track'), buttons: kSecondaryMouseButton);
    await tester.pumpAndSettle();

    expect(find.text('Remove from Playlist'), findsOneWidget);
    await tester.tap(find.text('Remove from Playlist'));
    await tester.pumpAndSettle();

    expect(removed, hasLength(1));
    expect(removed.single.index, 0);
    expect(removed.single.expectedRevision, Uint64.fromBigInt(BigInt.from(3)));
    updates.add(
      PlaylistUpdate(
        items: const [],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(4)),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Queue track'), findsNothing);
    expect(find.byType(Queue), findsOneWidget);

    switch (dismissal) {
      case _Dismissal.escape:
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      case _Dismissal.outsideClick:
        const outside = Offset(4, 4);
        expect(tester.getRect(find.byType(Queue)).contains(outside), isFalse);
        await tester.tapAt(outside, kind: PointerDeviceKind.mouse);
      case _Dismissal.coveredOwnerDisposal:
        unawaited(
          showDialog<void>(
            context: tester.element(find.byType(Queue)),
            builder: (_) => const ContentDialog(title: Text('Other dialog')),
          ),
        );
        await tester.pumpAndSettle();
        showOwner.value = false;
        await tester.pumpAndSettle();
        expect(find.text('Other dialog'), findsOneWidget);
        expect(find.byType(Queue), findsNothing);
        await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      case _Dismissal.ownerDisposal:
        showOwner.value = false;
    }
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byType(Queue), findsNothing);
    expect($history.history, history);

    showOwner.value = true;
    await tester.pumpAndSettle();
    await tester.tap(find.byType(QueueButton));
    await tester.pumpAndSettle();
    expect(find.byType(Queue), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(Queue), findsNothing);
    expect($history.history, history);
  });
}
