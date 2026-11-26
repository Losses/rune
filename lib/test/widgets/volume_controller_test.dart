import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../providers/volume.dart';
import '../../utils/system_volume.dart';
import '../../widgets/playback_controller/volume_button.dart';

class _System implements SystemVolume {
  @override
  Future<SystemVolumeState?> read() async =>
      const SystemVolumeState(volume: 0.5, settable: true);

  @override
  Future<bool> set(double volume) async => true;

  @override
  Stream<SystemVolumeState> get changes => const Stream.empty();
}

Future<VolumeProvider> _pump(
  WidgetTester tester, {
  required bool hosted,
  required TargetPlatform platform,
}) async {
  final provider = VolumeProvider(
    readStoredVolume: () async => null,
    writeStoredVolume: (_) async {},
    sendVolumeRequest: (_) {},
    systemVolume: _System(),
    hosted: HostedSignal(latest: hosted, changes: const Stream.empty()),
    platform: platform,
  );
  addTearDown(provider.dispose);
  await tester.pumpWidget(
    ChangeNotifierProvider.value(
      value: provider,
      child: const FluentApp(
        home: ScaffoldPage(
          content: VolumeController(width: 200, height: 40, vertical: false),
        ),
      ),
    ),
  );
  await tester.pump();
  return provider;
}

void main() {
  testWidgets(
    'the system volume view replaces the slider on iOS while hosted',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      await _pump(tester, hosted: true, platform: TargetPlatform.iOS);
      expect(find.byType(UiKitView), findsOneWidget);
      expect(find.byType(Slider), findsNothing);
      // Before the test ends, which checks that no debug override is left.
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('the slider stays on iOS for Rune\'s own volume', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    await _pump(tester, hosted: false, platform: TargetPlatform.iOS);
    expect(find.byType(UiKitView), findsNothing);
    expect(find.byType(Slider), findsOneWidget);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('the slider stays on macOS while hosted, and drives the system', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    final provider = await _pump(
      tester,
      hosted: true,
      platform: TargetPlatform.macOS,
    );
    expect(find.byType(UiKitView), findsNothing);
    expect(find.byType(Slider), findsOneWidget);
    expect(provider.usesSystemVolume, isTrue);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'the slider stays on Android while hosted, and drives the system',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      final provider = await _pump(
        tester,
        hosted: true,
        platform: TargetPlatform.android,
      );
      expect(find.byType(UiKitView), findsNothing);
      expect(find.byType(Slider), findsOneWidget);
      expect(provider.usesSystemVolume, isTrue);
      expect(provider.adjustable, isTrue);
      debugDefaultTargetPlatformOverride = null;
    },
  );
}
