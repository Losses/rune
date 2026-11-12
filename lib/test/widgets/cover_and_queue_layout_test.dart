import 'dart:async';
import 'dart:ui' as ui;

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playlist.dart';
import '../../providers/request_capabilities.dart';
import '../../providers/status.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/playback_controller/now_playing_track_cover_art_button.dart';
import '../../widgets/playback_controller/queue.dart';
import '../../widgets/tile/fancy_cover_painter.dart';

void main() {
  const layoutSizes = <Size>[
    Size(100, 800),
    Size(300, 700),
    Size(400, 800),
    Size(600, 900),
    Size(768, 1024),
    Size(1280, 120),
    Size(1280, 200),
    Size(1280, 280),
    Size(1280, 320),
  ];

  testWidgets('play bar cover and queue fit every required layout', (
    tester,
  ) async {
    for (final size in layoutSizes) {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = size;
      final updates = StreamController<PlaylistUpdate>();
      final failures = StreamController<RequestFailedResponse>();
      final playlist = PlaylistProvider(
        playlistUpdates: updates.stream,
        requestFailures: failures.stream,
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
              artist: 'A very long artist name that must shrink to fit',
              album: 'Album',
              title: 'A very long queue title that must shrink to fit',
              duration: 60,
            ),
          ],
          sessionId: 'session',
          revision: Uint64.fromBigInt(BigInt.from(1)),
        ),
      );

      await tester.pumpWidget(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<PlaylistProvider>.value(value: playlist),
            ChangeNotifierProvider(
              create: (_) => RequestCapabilitiesProvider(),
            ),
            ChangeNotifierProvider(create: (_) => PlaybackStatusProvider()),
          ],
          child: FluentApp(
            locale: const Locale('en'),
            localizationsDelegates: AppLocalizations.localizationsDelegates,
            supportedLocales: AppLocalizations.supportedLocales,
            home: RevealEffectContext(
              child: ScaffoldPage(
                content: SizedBox.expand(
                  child: Column(
                    children: [
                      const SizedBox.square(
                        dimension: 48,
                        child: NowPlayingTrackCoverArtButton(size: 48),
                      ),
                      Expanded(child: Queue()),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();

      expect(find.byType(NowPlayingTrackCoverArtButton), findsOneWidget);
      expect(
        tester.getSize(find.byType(NowPlayingTrackCoverArtButton)),
        const Size(48, 48),
        reason: 'cover size at $size',
      );
      expect(
        find.text('A very long queue title that must shrink to fit'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull, reason: 'layout $size');
    }
  });

  testWidgets('an infinite CoverArt paints text at its laid out size', (
    tester,
  ) async {
    await tester.pumpWidget(
      FluentApp(
        home: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(
              minWidth: 94,
              maxWidth: 94,
              minHeight: 96,
              maxHeight: 96,
            ),
            child: const CoverArt(
              path: null,
              size: double.infinity,
              hint: ('Album', 'Artist', 'Total Time 1:00'),
            ),
          ),
        ),
      ),
    );

    final customPaint = find.byWidgetPredicate(
      (widget) => widget is CustomPaint && widget.painter is FancyCoverPainter,
    );
    expect(tester.getSize(customPaint), const Size(94, 96));
    final painter =
        tester.widget<CustomPaint>(customPaint).painter! as FancyCoverPainter;
    final paintSize = tester.getSize(customPaint);
    final recorder = ui.PictureRecorder();
    painter.paint(ui.Canvas(recorder), paintSize);
    final picture = recorder.endRecording();
    final blankTextPainter = FancyCoverPainter(
      texts: List.filled(painter.texts.length, ''),
      configs: painter.configs,
      color: painter.color,
    );
    final blankRecorder = ui.PictureRecorder();
    blankTextPainter.paint(ui.Canvas(blankRecorder), paintSize);
    final blankPicture = blankRecorder.endRecording();
    expect(
      picture.approximateBytesUsed,
      greaterThan(blankPicture.approximateBytesUsed),
    );
    picture.dispose();
    blankPicture.dispose();
    expect(tester.takeException(), isNull);
  });
}
