import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/status.dart';
import '../../widgets/playback_controller/queue.dart';

void main() {
  Future<void> pumpQueue(
    WidgetTester tester, {
    int? current,
    int? laterCurrent,
    int count = 100,
    bool Function(int index)? unavailable,
    List<MovePlaylistItemRequest>? moves,
  }) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(400, 400);
    addTearDown(tester.view.reset);

    final updates = StreamController<PlaylistUpdate>();
    final failures = StreamController<RequestFailedResponse>();
    final statuses = StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
      sync: true,
    );
    final playlist = PlaylistProvider(
      playlistUpdates: updates.stream,
      requestFailures: failures.stream,
      sendMoveRequest: moves?.add,
    );
    final status = PlaybackStatusProvider(statusStream: statuses.stream);
    addTearDown(() async {
      playlist.dispose();
      status.dispose();
      await updates.close();
      await failures.close();
      await statuses.close();
    });

    updates.add(
      PlaylistUpdate(
        items: [
          for (var i = 0; i < count; i++)
            PlaylistItem(
              unavailable: unavailable != null && unavailable(i)
                  ? PlaylistUnavailableReason.unreadable
                  : null,
              item: PlayingItemRefIndependentFile(path: 'p$i'),
              artist: 'Artist',
              album: 'Album',
              title: 'Track $i',
              duration: 60,
            ),
        ],
        sessionId: 'session',
        revision: Uint64.fromBigInt(BigInt.from(1)),
      ),
    );
    void emit(int index) {
      statuses.add(
        RustSignalPack(
          PlaybackStatus(
            sessionId: 'local',
            state: PlaybackState.playing,
            progressSeconds: 0,
            progressPercentage: 0,
            duration: 60,
            index: index,
            itemRequest: PlayingItemRefIndependentFile(path: 'p$index'),
            playbackMode: PlaybackMode.sequential,
            ready: true,
            libPath: '',
          ),
          Uint8List(0),
        ),
      );
    }

    if (current != null) emit(current);
    await tester.pump();
    await tester.pump();

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<PlaylistProvider>.value(value: playlist),
          ChangeNotifierProvider(create: (_) => RequestCapabilitiesProvider()),
          ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
        ],
        child: FluentApp(
          locale: const Locale('en'),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: const ScaffoldPage(content: SizedBox.expand(child: Queue())),
        ),
      ),
    );
    await tester.pumpAndSettle();
    if (laterCurrent != null) {
      emit(laterCurrent);
      await tester.pumpAndSettle();
    }
  }

  double offset(WidgetTester tester) =>
      tester.state<ScrollableState>(find.byType(Scrollable)).position.pixels;

  testWidgets('the playing row is in view when the queue opens', (
    tester,
  ) async {
    await pumpQueue(tester, current: 60);

    expect(find.text('Track 60'), findsOneWidget);
    // Rows in the sliver cache are built but clipped, so the check is against
    // the scrollable's own viewport, not the screen.
    final viewport = tester.getRect(find.byType(Scrollable));
    final rect = tester.getRect(find.text('Track 60'));
    expect(rect.top, greaterThanOrEqualTo(viewport.top));
    expect(rect.bottom, lessThanOrEqualTo(viewport.bottom));
    expect(offset(tester), greaterThan(0));
  });

  testWidgets('the queue stays at the top when the first row plays', (
    tester,
  ) async {
    await pumpQueue(tester, current: 0);

    expect(offset(tester), 0);
  });

  testWidgets('the queue stays at the top when nothing plays', (tester) async {
    await pumpQueue(tester);

    expect(offset(tester), 0);
  });

  testWidgets('the playing row is revealed when the status arrives after the '
      'first build', (tester) async {
    await pumpQueue(tester, laterCurrent: 60);

    final viewport = tester.getRect(find.byType(Scrollable));
    final rect = tester.getRect(find.text('Track 60'));
    expect(rect.top, greaterThanOrEqualTo(viewport.top));
    expect(rect.bottom, lessThanOrEqualTo(viewport.bottom));
    expect(offset(tester), greaterThan(0));
  });

  testWidgets('the playing row is reached when rows differ in height', (
    tester,
  ) async {
    await pumpQueue(
      tester,
      current: 650,
      count: 1000,
      unavailable: (i) => i >= 10 && i < 640,
    );

    final viewport = tester.getRect(find.byType(Scrollable));
    expect(find.text('Track 650'), findsOneWidget);
    final rect = tester.getRect(find.text('Track 650'));
    expect(rect.top, greaterThanOrEqualTo(viewport.top));
    expect(rect.bottom, lessThanOrEqualTo(viewport.bottom));
  });

  testWidgets('the playing row can be dragged once it is revealed', (
    tester,
  ) async {
    final moves = <MovePlaylistItemRequest>[];
    await pumpQueue(tester, current: 1, count: 10, moves: moves);

    // The drag feedback of reorderables is a card no taller than the row, so
    // the row overflows inside it. That overflow is not what this test checks.
    final errors = <String>[];
    final onError = FlutterError.onError;
    FlutterError.onError = (details) {
      final error = details.exceptionAsString();
      if (!error.contains('RenderFlex overflowed')) errors.add(error);
    };
    try {
      // A mouse on desktop, because on a touch platform a long press opens the
      // row's context menu instead of dragging it.
      final gesture = await tester.startGesture(
        tester.getCenter(find.text('Track 1')),
        kind: PointerDeviceKind.mouse,
      );
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 100));
      await gesture.moveBy(const Offset(0, 60));
      await tester.pump();
      await gesture.moveBy(const Offset(0, 60));
      await tester.pump(const Duration(milliseconds: 300));
      await gesture.up();
      await tester.pumpAndSettle();
    } finally {
      FlutterError.onError = onError;
    }

    expect(errors, isEmpty);
    expect(moves.map((move) => move.oldIndex), [1]);
  }, variant: TargetPlatformVariant.only(TargetPlatform.macOS));
}
