import 'dart:io';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_staggered_grid_view/flutter_staggered_grid_view.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/playback_controller.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/providers/volume.dart';
import 'package:rune/screens/settings_laboratory/settings_laboratory.dart';
import 'package:rune/screens/settings_media_controller/settings_media_controller.dart';
import 'package:rune/utils/settings_manager.dart';
import 'package:rune/utils/storage_key_manager.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/widgets/playback_controller/constants/controller_items.dart';
import 'package:rune/widgets/playback_controller/constants/playback_controller_height.dart';
import 'package:rune/widgets/rune_clickable.dart';

// An iPhone's status bar and home indicator, in points.
const _iPhone = EdgeInsets.only(top: 62, bottom: 34);

// A window with the zune layout, whose cover art disk takes a third of its
// width at the bottom, and one with the phone layout and its controller.
const _zune = Size(340, 700);
const _phone = Size(390, 844);

class _Controller extends ChangeNotifier implements PlaybackControllerProvider {
  @override
  List<ControllerEntry> availableEntries(BuildContext context) => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Volume extends ChangeNotifier implements VolumeProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Scenes extends ChangeNotifier implements VisualizerScenesProvider {
  @override
  bool? get gpu => false;

  @override
  bool get log => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

Future<void> _open(WidgetTester tester, Size size, Widget page) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  const insets = FakeViewPadding(top: 62, bottom: 34);
  tester.view.viewPadding = insets;
  tester.view.padding = insets;
  addTearDown(tester.view.reset);
  final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
  final responsive = ResponsiveProvider(sizes, () async => null);
  addTearDown(sizes.dispose);
  addTearDown(responsive.dispose);

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider.value(value: sizes),
        ChangeNotifierProvider.value(value: responsive),
        ChangeNotifierProvider<PlaybackControllerProvider>.value(
          value: _Controller(),
        ),
        ChangeNotifierProvider<VolumeProvider>.value(value: _Volume()),
        ChangeNotifierProvider<VisualizerScenesProvider>.value(
          value: _Scenes(),
        ),
      ],
      child: FluentApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: page,
      ),
    ),
  );
  // Lets the saved settings load and the window size debounce settle.
  await tester.pump(const Duration(seconds: 2));
}

EdgeInsets _framePadding(WidgetTester tester) =>
    tester
            .widget<Container>(
              find
                  .descendant(
                    of: find.byType(PageContentFrame),
                    matching: find.byType(Container),
                  )
                  .first,
            )
            .padding!
        as EdgeInsets;

void main() {
  late Directory storage;
  setUpAll(() async {
    storage = await Directory.systemTemp.createTemp('rune-settings-test');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('plugins.flutter.io/path_provider'),
          (_) async => storage.path,
        );
    StorageKeyManager.initialize(null);
    await SettingsManager().getValue<String>('settings-page-frame-test');
  });
  tearDownAll(() async {
    if (!Platform.isWindows) await storage.delete(recursive: true);
  });

  // The pages run under the home indicator, so its inset is never added; the
  // iPhone's rule for the bottom is iOS's.
  final iOS = TargetPlatformVariant.only(TargetPlatform.iOS);

  group('the media controller settings', () {
    EdgeInsetsGeometry? listPadding(WidgetTester tester) => tester
        .widget<SingleChildScrollView>(
          find
              .descendant(
                of: find.byType(SettingsMediaControllerPage),
                matching: find.byType(SingleChildScrollView),
              )
              .first,
        )
        .padding;

    testWidgets('keep the list clear of the cover art disk', (tester) async {
      await _open(tester, _zune, const SettingsMediaControllerPage());

      expect(listPadding(tester), const EdgeInsets.only(bottom: 340 / 3));
    }, variant: iOS);

    testWidgets('end above the controller', (tester) async {
      await _open(tester, _phone, const SettingsMediaControllerPage());

      expect(listPadding(tester), EdgeInsets.zero);
      expect(_framePadding(tester).bottom, playbackControllerHeight);
    }, variant: iOS);
  });

  group('the laboratory', () {
    EdgeInsetsGeometry? gridPadding(WidgetTester tester) =>
        tester.widget<MasonryGridView>(find.byType(MasonryGridView)).padding;

    testWidgets('keeps the grid clear of the cover art disk', (tester) async {
      await _open(tester, _zune, const SettingsLaboratory());

      expect(gridPadding(tester), const EdgeInsets.only(top: 4, bottom: 340 / 3));
      // Below the status bar, as before.
      final back = find.ancestor(
        of: find.byIcon(Symbols.arrow_back),
        matching: find.byType(RuneClickable),
      );
      expect(tester.getTopLeft(back), Offset(16, _iPhone.top + 16));
    }, variant: iOS);

    testWidgets('ends the grid above the controller', (tester) async {
      await _open(tester, _phone, const SettingsLaboratory());

      expect(gridPadding(tester), const EdgeInsets.only(top: 4));
      expect(_framePadding(tester).bottom, playbackControllerHeight);
      expect(_framePadding(tester).top, 0);
    }, variant: iOS);
  });
}
