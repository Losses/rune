import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../providers/status.dart';
import '../../widgets/ax_reveal/widgets/reveal_effect_context.dart';
import '../../widgets/playback_controller/now_playing_track_cover_art_button.dart';
import '../../widgets/tile/cover_art.dart';

PlaybackStatus _jukeboxStatus({String? coverArtPath}) => PlaybackStatus(
  sessionId: 'subsonic-library',
  state: PlaybackState.playing,
  progressSeconds: 1,
  progressPercentage: 0.1,
  title: 'Song',
  duration: 10,
  playbackMode: PlaybackMode.sequential,
  ready: true,
  coverArtPath: coverArtPath,
  libPath: '',
);

void main() {
  testWidgets(
    'the play bar cover shows the cover a later jukebox status carries',
    (tester) async {
      final directory = Directory.systemTemp.createTempSync('jukebox-cover');
      addTearDown(() => directory.deleteSync(recursive: true));
      final cover = File('${directory.path}/cover.img')
        ..writeAsBytesSync(
          base64Decode(
            'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
          ),
        );
      final messages =
          StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
            sync: true,
          );
      final statusProvider = PlaybackStatusProvider(
        statusStream: messages.stream,
      );
      addTearDown(statusProvider.dispose);
      addTearDown(messages.close);

      await tester.pumpWidget(
        ChangeNotifierProvider<PlaybackStatusProvider>.value(
          value: statusProvider,
          child: FluentApp(
            home: RevealEffectContext(
              child: const ScaffoldPage(
                content: Center(child: NowPlayingTrackCoverArtButton(size: 48)),
              ),
            ),
          ),
        ),
      );

      messages.add(RustSignalPack(_jukeboxStatus(), Uint8List(0)));
      await tester.pump();
      await tester.pump();
      expect(tester.widget<CoverArt>(find.byType(CoverArt)).path, isNull);

      messages.add(
        RustSignalPack(_jukeboxStatus(coverArtPath: cover.path), Uint8List(0)),
      );
      await tester.pump();
      await tester.pump();
      await tester.pump();
      expect(tester.widget<CoverArt>(find.byType(CoverArt)).path, cover.path);
    },
  );
}
