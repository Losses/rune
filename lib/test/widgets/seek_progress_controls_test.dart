import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/playback_controller.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/status.dart';
import '../../providers/visualizer_lab.dart';
import '../../utils/api/seek_absolute.dart';
import '../../screens/cover_wall/widgets/cover_art_page_progress_bar.dart';
import '../../widgets/playback_controller/constants/controller_items.dart';
import '../../widgets/playback_controller/conrtoller_progress_bar.dart';

class _Controller extends ChangeNotifier implements PlaybackControllerProvider {
  @override
  List<ControllerEntry> availableEntries(BuildContext context) => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Responsive extends ChangeNotifier implements ResponsiveProvider {
  @override
  bool smallerOrEqualTo(DeviceType type, [bool strictOrientation = true]) =>
      false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Lab extends ChangeNotifier implements VisualizerLabProvider {
  @override
  bool get enabled => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() => registerSeekProgressControlsTest();

/// Registers the shared seek-control scenario.
///
/// [verifyNoSeek] can observe the native receiver while the test exercises
/// the disabled sliders and their saved drag callbacks.
void registerSeekProgressControlsTest({
  Future<void> Function(WidgetTester tester, Future<void> Function() attempt)?
  verifyNoSeek,
}) {
  testWidgets('both seek sliders disable when the sequential queue stops', (
    tester,
  ) async {
    final messages = StreamController<RustSignalPack<PlaybackStatus>>.broadcast(
      sync: true,
    );
    addTearDown(messages.close);
    final status = PlaybackStatusProvider(statusStream: messages.stream);
    addTearDown(status.dispose);

    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider.value(value: status),
          ChangeNotifierProvider<ResponsiveProvider>(
            create: (_) => _Responsive(),
          ),
          ChangeNotifierProvider<PlaybackControllerProvider>(
            create: (_) => _Controller(),
          ),
          ChangeNotifierProvider<VisualizerLabProvider>(create: (_) => _Lab()),
        ],
        child: FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Consumer<PlaybackStatusProvider>(
            builder: (context, provider, child) => Center(
              child: SizedBox(
                width: 360,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    SizedBox(
                      height: 120,
                      child: ControllerProgressBar(
                        item: null,
                        status: provider.playbackStatus,
                      ),
                    ),
                    const SizedBox(
                      height: 120,
                      child: CoverArtPageProgressBar(shadows: []),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );

    Future<void> deliver(PlaybackState state, {required bool ready}) async {
      messages.add(
        RustSignalPack(
          PlaybackStatus(
            sessionId: 'local',
            state: state,
            progressSeconds: state == PlaybackState.stopped ? 10 : 3,
            progressPercentage: state == PlaybackState.stopped ? 1 : 0.3,
            duration: 10,
            playbackMode: PlaybackMode.sequential,
            ready: ready,
            libPath: '/music',
          ),
          Uint8List(0),
        ),
      );
      await tester.pump();
    }

    final sliders = find.byType(Slider);
    List<bool> enabledControls() {
      expect(sliders, findsNWidgets(2));
      return tester
          .widgetList<Slider>(sliders)
          .map((slider) => slider.onChanged != null)
          .toList();
    }

    await deliver(PlaybackState.playing, ready: true);
    expect(enabledControls(), [true, true]);
    await deliver(PlaybackState.paused, ready: true);
    expect(enabledControls(), [true, true]);
    await deliver(PlaybackState.playing, ready: true);
    final activeCallbacks = tester
        .widgetList<Slider>(sliders)
        .map((slider) => slider.onChanged!)
        .toList();
    await deliver(PlaybackState.stopped, ready: false);
    expect(status.playbackStatus.ready, isTrue, reason: 'retained status');
    expect(enabledControls(), [
      false,
      false,
    ], reason: 'a stopped queue must disable both seek sliders');
    Future<void> attemptSeeks() async {
      for (var index = 0; index < 2; index++) {
        await tester.tap(sliders.at(index), warnIfMissed: false);
        await tester.pump();
      }
      // A drag can still hold a callback from before the stopped update.
      for (final callback in activeCallbacks) {
        expect(() => callback(50), returnsNormally);
      }
      expect(seekAbsolute(5, status.playbackStatus), isFalse);
    }

    if (verifyNoSeek == null) {
      // Sending a request would fail in the widget harness without a bridge.
      await attemptSeeks();
    } else {
      await verifyNoSeek(tester, attemptSeeks);
    }
    expect(tester.takeException(), isNull);

    await deliver(PlaybackState.paused, ready: false);
    expect(enabledControls(), [false, false]);
    await deliver(PlaybackState.paused, ready: true);
    expect(enabledControls(), [true, true]);
  });
}
