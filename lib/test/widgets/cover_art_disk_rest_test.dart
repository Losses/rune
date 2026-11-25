import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/status.dart';
import '../../widgets/playback_controller/cover_art_disk.dart';

class _Status extends ChangeNotifier implements PlaybackStatusProvider {
  @override
  final playbackStatus = PlaybackStatusState();

  @override
  bool get notReady => false;

  void set(PlaybackState state) {
    playbackStatus.state = state;
    notifyListeners();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

// The car layout, where the disk peeks in from the right edge.
Future<_Status> _pumpDisk(WidgetTester tester) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(800, 320);
  addTearDown(tester.view.reset);

  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  final status = _Status()..playbackStatus.state = PlaybackState.paused;
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);
  addTearDown(status.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: sizes),
        ChangeNotifierProvider.value(value: responsive),
        ChangeNotifierProvider<PlaybackStatusProvider>.value(value: status),
      ],
      child: FluentTheme(
        data: FluentThemeData(),
        child: const Directionality(
          textDirection: TextDirection.ltr,
          child: Stack(children: [CoverArtDisk()]),
        ),
      ),
    ),
  );
  return status;
}

Future<void> _frames(WidgetTester tester, int count) async {
  for (var i = 0; i < count; i++) {
    await tester.pump(const Duration(milliseconds: 16));
  }
}

double _angle(WidgetTester tester) {
  final transform = tester
      .widget<Transform>(
        find
            .descendant(
              of: find.byType(CoverArtDisk),
              matching: find.byType(Transform),
            )
            .first,
      )
      .transform;
  return atan2(transform.entry(1, 0), transform.entry(0, 0));
}

void main() {
  testWidgets('the disk asks for no frames once it has come to rest', (
    tester,
  ) async {
    await _pumpDisk(tester);

    // It slides in while it is paused, then stays where it is.
    expect(tester.binding.hasScheduledFrame, isTrue);
    await _frames(tester, 300);
    expect(tester.binding.hasScheduledFrame, isFalse);

    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('the disk turns while music plays and stops after a pause', (
    tester,
  ) async {
    final status = await _pumpDisk(tester);
    await _frames(tester, 300);
    final restingAngle = _angle(tester);

    status.set(PlaybackState.playing);
    await tester.pump();
    expect(tester.binding.hasScheduledFrame, isTrue);
    await _frames(tester, 120);
    expect(_angle(tester), isNot(closeTo(restingAngle, 1e-3)));

    status.set(PlaybackState.paused);
    await _frames(tester, 400);
    expect(tester.binding.hasScheduledFrame, isFalse);
    final pausedAngle = _angle(tester);
    await _frames(tester, 30);
    expect(_angle(tester), pausedAngle);

    await tester.pumpWidget(const SizedBox());
  });
}
