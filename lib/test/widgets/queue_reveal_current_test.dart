import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
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
  Future<void> pumpQueue(WidgetTester tester, {int? current}) async {
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
          for (var i = 0; i < 100; i++)
            PlaylistItem(
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
    if (current != null) {
      statuses.add(
        RustSignalPack(
          PlaybackStatus(
            sessionId: 'local',
            state: PlaybackState.playing,
            progressSeconds: 0,
            progressPercentage: 0,
            duration: 60,
            index: current,
            itemRequest: PlayingItemRefIndependentFile(path: 'p$current'),
            playbackMode: PlaybackMode.sequential,
            ready: true,
            libPath: '',
          ),
          Uint8List(0),
        ),
      );
    }
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
    await tester.pump();
    await tester.pump();
    await tester.pump();
  }

  double offset(WidgetTester tester) =>
      tester.state<ScrollableState>(find.byType(Scrollable)).position.pixels;

  testWidgets('the playing row is in view when the queue opens', (
    tester,
  ) async {
    await pumpQueue(tester, current: 60);

    expect(find.text('Track 60'), findsOneWidget);
    final rect = tester.getRect(find.text('Track 60'));
    expect(rect.top, greaterThanOrEqualTo(0));
    expect(rect.bottom, lessThanOrEqualTo(400));
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
}
