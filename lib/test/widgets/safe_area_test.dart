import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/l10n/app_localizations.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/providers/library_manager.dart';
import 'package:rune/providers/library_path.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/providers/status.dart';
import 'package:rune/screens/home/home.dart';
import 'package:rune/screens/settings_library/widgets/add_library_setting_button.dart';
import 'package:rune/utils/kept_clear.dart';
import 'package:rune/widgets/app_viewport.dart';
import 'package:rune/widgets/keep_clear.dart';
import 'package:rune/widgets/navigation_bar/constants/navigation_bar_height.dart';
import 'package:rune/widgets/navigation_bar/page_chrome.dart';
import 'package:rune/widgets/navigation_bar/page_content_frame.dart';
import 'package:rune/widgets/playback_controller/constants/playback_controller_height.dart';
import 'package:rune/widgets/playback_controller/cover_art_disk.dart';
import 'package:rune/widgets/settings/settings_button.dart';
import 'package:rune/widgets/status_bar_style.dart';

// An iPhone upright and on its side, in points.
const _portrait = EdgeInsets.only(top: 62, bottom: 34);
const _landscape = EdgeInsets.only(left: 62, right: 62, bottom: 21);

const _phone = Size(390, 844);

FakeViewPadding _fake(EdgeInsets insets) => FakeViewPadding(
  left: insets.left,
  top: insets.top,
  right: insets.right,
  bottom: insets.bottom,
);

// A window of [size] at a device pixel ratio of 1, with the system's
// [viewPadding] and [viewInsets] (the keyboard), and Rune's providers for it.
Future<(ScreenSizeProvider, ResponsiveProvider)> _pump(
  WidgetTester tester, {
  required Size size,
  EdgeInsets viewPadding = EdgeInsets.zero,
  EdgeInsets viewInsets = EdgeInsets.zero,
  bool settle = true,
  required Widget child,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  tester.view.viewPadding = _fake(viewPadding);
  // As the system has it: the padding is what the keyboard leaves uncovered.
  tester.view.padding = _fake(
    viewPadding.copyWith(
      bottom: (viewPadding.bottom - viewInsets.bottom).clamp(
        0,
        double.infinity,
      ),
    ),
  );
  tester.view.viewInsets = _fake(viewInsets);
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
      ],
      child: FluentTheme(
        data: FluentThemeData(),
        child: Directionality(textDirection: TextDirection.ltr, child: child),
      ),
    ),
  );
  // Lets the window size debounce settle.
  if (settle) await tester.pump(const Duration(seconds: 2));
  return (sizes, responsive);
}

EdgeInsets _framePadding(WidgetTester tester) {
  final container = tester.widget<Container>(
    find.descendant(
      of: find.byType(PageContentFrame),
      matching: find.byType(Container),
    ),
  );
  return container.padding! as EdgeInsets;
}

// The media query the content of a page sees, and where the content is.
late MediaQueryData _inside;
const _content = Key('content');

Widget _probe() {
  return Builder(
    builder: (context) {
      _inside = MediaQuery.of(context);
      return const SizedBox.expand(key: _content);
    },
  );
}

late EdgeInsets _scrollPadding;

Widget _scrollPaddingProbe({double leftPlus = 0, double rightPlus = 0}) {
  return Builder(
    builder: (context) {
      _scrollPadding = getScrollContainerPadding(
        context,
        leftPlus: leftPlus,
        rightPlus: rightPlus,
      );
      return const SizedBox();
    },
  );
}

void main() {
  test('the bottom is never kept clear of', () {
    for (final insets in const [
      EdgeInsets.fromLTRB(0, 62, 0, 34),
      EdgeInsets.fromLTRB(62, 0, 62, 21),
      EdgeInsets.fromLTRB(0, 24, 0, 48),
    ]) {
      final clear = keptClearOf(insets);
      expect(clear.bottom, 0);
      expect(clear.copyWith(bottom: insets.bottom), insets);
    }
  });

  group('ScreenSizeProvider', () {
    testWidgets('chooses the layout by the width between the side insets', (
      tester,
    ) async {
      // An iPhone 13 mini in landscape: 812 x 375 points at 3x.
      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(2436, 1125);
      addTearDown(tester.view.reset);
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      final responsive = ResponsiveProvider(sizes, () async => null);
      addTearDown(sizes.dispose);
      addTearDown(responsive.dispose);
      expect(responsive.equalTo(DeviceType.desktop), isTrue);

      tester.view.viewPadding = const FakeViewPadding(
        left: 150,
        right: 150,
        bottom: 63,
      );
      await tester.pump();
      expect(sizes.layoutSize, const Size(712, 375));
      // The real size stays the whole window.
      expect(sizes.screenSize, const Size(812, 375));
      expect(responsive.equalTo(DeviceType.tablet), isTrue);
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('leaves out the status bar but not the home indicator', (
      tester,
    ) async {
      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(1125, 2436);
      tester.view.viewPadding = const FakeViewPadding(top: 150, bottom: 102);
      addTearDown(tester.view.reset);
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      addTearDown(sizes.dispose);
      expect(sizes.layoutSize, const Size(375, 762));
      expect(sizes.screenSize, const Size(375, 812));
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('a rotation notifies once', (tester) async {
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = _phone;
      tester.view.viewPadding = _fake(_portrait);
      addTearDown(tester.view.reset);
      final sizes = ScreenSizeProvider.withWriter(null, (_) async {});
      addTearDown(sizes.dispose);
      var notified = 0;
      sizes.addListener(() => notified++);

      // The engine reports a rotation, the new size and insets, as one
      // change of the metrics; the test view reports each of them, so the
      // provider hears of them after both have changed.
      WidgetsBinding.instance.removeObserver(sizes);
      tester.view.physicalSize = _phone.flipped;
      tester.view.viewPadding = _fake(_landscape);
      WidgetsBinding.instance.addObserver(sizes);
      sizes.didChangeMetrics();
      expect(notified, 1);
      expect(sizes.layoutSize, const Size(844 - 124, 390));

      // The insets alone also count, and nothing new is nothing.
      tester.view.viewPadding = _fake(_landscape.copyWith(left: 0));
      expect(notified, 2);
      sizes.didChangeMetrics();
      expect(notified, 2);
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('under the app viewport, the layout size is the media '
        'query\'s size less the insets kept clear of', (tester) async {
      // An iPhone 15 Pro on its side, 852 x 393 points at 3x.
      tester.view.devicePixelRatio = 3;
      tester.view.physicalSize = const Size(2556, 1179);
      tester.view.viewPadding = _fake(_landscape * 3);
      tester.view.padding = _fake(_landscape * 3);
      addTearDown(tester.view.reset);
      final scale = AppScaleProvider(
        read: () async => 125,
        write: (_) async {},
      );
      await scale.initialize();
      addTearDown(scale.dispose);
      final sizes = ScreenSizeProvider.withWriter(scale, (_) async {});
      addTearDown(sizes.dispose);

      late MediaQueryData media;
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: AppViewport(
            child: Builder(
              builder: (context) {
                media = MediaQuery.of(context);
                return const SizedBox.expand();
              },
            ),
          ),
        ),
      );

      final clear = keptClearOf(media.viewPadding);
      expect(
        sizes.layoutSize.width,
        moreOrLessEquals(media.size.width - clear.horizontal),
      );
      expect(
        sizes.layoutSize.height,
        moreOrLessEquals(media.size.height - clear.top),
      );
      expect(sizes.screenSize, media.size);
      await tester.pump(const Duration(seconds: 2));
    });

    testWidgets('a 360 x 640 Android phone on its side takes the car layout '
        'from the height under its status bar', (tester) async {
      // Pinned on purpose: the layout follows the room under the status bar
      // on every platform, so this window is car, not mobile, with the bar.
      final (sizes, responsive) = await _pump(
        tester,
        size: const Size(640, 360),
        viewPadding: const EdgeInsets.only(top: 24),
        child: const SizedBox(),
      );

      expect(sizes.layoutSize, const Size(640, 336));
      expect(responsive.currentDeviceType, DeviceType.car);

      tester.view.viewPadding = FakeViewPadding.zero;
      expect(responsive.currentDeviceType, DeviceType.mobile);
    });
  });

  group('pageChromeOf', () {
    // What the navigation bar, the controller and the disk take in each
    // layout, as the page frame and the lists reserved before.
    final cases = <(String, Size, DeviceType, double, double, EdgeInsets)>[
      (
        'dock',
        const Size(100, 400),
        DeviceType.dock,
        bandNavigationBarHeight,
        0,
        const EdgeInsets.only(bottom: 100 / 3),
      ),
      (
        'zune',
        const Size(240, 400),
        DeviceType.zune,
        fullNavigationBarHeight,
        0,
        const EdgeInsets.only(bottom: 80),
      ),
      (
        'phone',
        _phone,
        DeviceType.phone,
        fullNavigationBarHeight,
        playbackControllerHeight,
        EdgeInsets.zero,
      ),
      (
        'desktop',
        const Size(1280, 800),
        DeviceType.desktop,
        fullNavigationBarHeight,
        playbackControllerHeight,
        EdgeInsets.zero,
      ),
      (
        'band',
        const Size(800, 100),
        DeviceType.band,
        0,
        0,
        const EdgeInsets.only(right: 100 / 3),
      ),
      (
        'fish',
        const Size(800, 280),
        DeviceType.fish,
        carNavigationBarHeight,
        0,
        const EdgeInsets.only(right: 280 / 3),
      ),
      (
        'car',
        const Size(800, 320),
        DeviceType.car,
        carNavigationBarHeight,
        0,
        const EdgeInsets.only(right: 320 / 3),
      ),
    ];

    for (final (name, size, type, header, controller, disk) in cases) {
      testWidgets('takes what it took before in $name', (tester) async {
        final (sizes, responsive) = await _pump(
          tester,
          size: size,
          child: const SizedBox(),
        );
        expect(responsive.currentDeviceType, type);

        final chrome = pageChromeOf(responsive, sizes.screenSize);
        expect(chrome.header, header);
        expect(chrome.controller, controller);
        expect(chrome.disk, disk);
      });
    }
  });

  group('PageContentFrame', () {
    testWidgets('the top follows the header and the status bar', (
      tester,
    ) async {
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(child: _probe()),
      );

      expect(
        _framePadding(tester).top,
        fullNavigationBarHeight + _portrait.top,
      );
    });

    testWidgets('the band layout keeps clear of the status bar too', (
      tester,
    ) async {
      await _pump(
        tester,
        size: const Size(800, 140),
        viewPadding: const EdgeInsets.only(top: 30),
        child: PageContentFrame(child: _probe()),
      );

      expect(_framePadding(tester).top, 30);
      // Room for the back button, as before.
      expect(_framePadding(tester).left, 24);
    });

    testWidgets('the bottom clears the controller, not the home indicator', (
      tester,
    ) async {
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(child: _probe()),
      );

      expect(_framePadding(tester).bottom, playbackControllerHeight);
    });

    testWidgets('the keyboard still pushes the content up', (tester) async {
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        viewInsets: const EdgeInsets.only(bottom: 300),
        child: PageContentFrame(child: _probe()),
      );

      expect(_framePadding(tester).bottom, playbackControllerHeight + 300);
      // The frame has taken the keyboard.
      expect(_inside.viewInsets.bottom, 0);
    });

    testWidgets('the content sees what is left: nothing on a phone upright', (
      tester,
    ) async {
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(child: _probe()),
      );

      expect(_inside.padding, EdgeInsets.zero);
      expect(_inside.viewPadding, EdgeInsets.zero);
    });

    testWidgets('the content sees the status bar when it has the top', (
      tester,
    ) async {
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(top: false, child: _probe()),
      );

      expect(_framePadding(tester).top, 0);
      expect(_inside.padding, EdgeInsets.only(top: _portrait.top));
      expect(_inside.viewPadding, _inside.padding);
    });

    testWidgets('the content sees the sides on a phone on its side', (
      tester,
    ) async {
      await _pump(
        tester,
        size: _phone.flipped,
        viewPadding: _landscape,
        child: PageContentFrame(child: _probe()),
      );

      const sides = EdgeInsets.only(left: 62, right: 62);
      expect(_inside.padding, sides);
      expect(_inside.viewPadding, sides);
    });

    testWidgets(
      'the content sees the disk and the keyboard in the car layout',
      (tester) async {
        await _pump(
          tester,
          size: const Size(640, 360),
          viewPadding: const EdgeInsets.only(top: 24, right: 48),
          viewInsets: const EdgeInsets.only(bottom: 200),
          child: PageContentFrame(child: _probe()),
        );

        const remaining = EdgeInsets.fromLTRB(0, 0, 48 + 360 / 3, 200);
        expect(_framePadding(tester).top, carNavigationBarHeight + 24);
        expect(_framePadding(tester).bottom, 0);
        expect(_inside.padding, remaining);
        expect(_inside.viewPadding, remaining);
        expect(_inside.viewInsets.bottom, 0);
      },
    );

    testWidgets('a list inside does not add the insets again', (tester) async {
      const first = Key('first');
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(
          child: ListView(children: const [SizedBox(key: first, height: 10)]),
        ),
      );

      expect(
        tester.getTopLeft(find.byKey(first)).dy,
        fullNavigationBarHeight + _portrait.top,
      );
    });

    testWidgets('a scroll bar along the bottom of a list stays at its bottom', (
      tester,
    ) async {
      final controller = ScrollController();
      addTearDown(controller.dispose);
      await _pump(
        tester,
        size: _phone,
        viewPadding: _portrait,
        child: PageContentFrame(
          child: Scrollbar(
            controller: controller,
            thumbVisibility: true,
            child: ListView(
              controller: controller,
              scrollDirection: Axis.horizontal,
              children: const [SizedBox(width: 2000)],
            ),
          ),
        ),
      );

      final painter = tester
          .widgetList<CustomPaint>(
            find.descendant(
              of: find.byType(Scrollbar),
              matching: find.byType(CustomPaint),
            ),
          )
          .map((paint) => paint.foregroundPainter)
          .whereType<ScrollbarPainter>()
          .single;
      // Fluent's own margin, without the home indicator under it.
      expect(painter.padding.resolve(TextDirection.ltr).bottom, 4);
    });
  });

  group('getScrollContainerPadding', () {
    testWidgets('keeps clear of the notch in landscape', (tester) async {
      await _pump(
        tester,
        size: _phone.flipped,
        viewPadding: _landscape,
        child: PageContentFrame(child: _scrollPaddingProbe()),
      );

      expect(_scrollPadding.left, _landscape.left);
      expect(_scrollPadding.right, _landscape.right);
      expect(_scrollPadding.bottom, 0);
    });

    testWidgets('leaves the zune list bottom to the disk', (tester) async {
      await _pump(
        tester,
        size: const Size(240, 400),
        viewPadding: _portrait,
        child: PageContentFrame(child: _scrollPaddingProbe()),
      );

      expect(_scrollPadding.bottom, 240 / 3);
    });

    testWidgets('the keyboard still lifts the zune list', (tester) async {
      await _pump(
        tester,
        size: const Size(240, 400),
        viewPadding: _portrait,
        viewInsets: const EdgeInsets.only(bottom: 200),
        child: PageContentFrame(child: _scrollPaddingProbe()),
      );

      expect(_scrollPadding.bottom, 240 / 3 + 200);
    });

    testWidgets('keeps the belt list off the left by its own left', (
      tester,
    ) async {
      await _pump(
        tester,
        size: const Size(800, 200),
        child: PageContentFrame(
          child: _scrollPaddingProbe(leftPlus: 12, rightPlus: 30),
        ),
      );

      expect(_scrollPadding.left, 16 + 12);
      expect(_scrollPadding.right, 200 / 3 + 30);
    });
  });

  group('KeepClear', () {
    testWidgets('takes the sides it keeps clear of out of both paddings', (
      tester,
    ) async {
      const insets = EdgeInsets.fromLTRB(47, 59, 47, 34);
      await _pump(
        tester,
        size: _phone.flipped,
        viewPadding: insets,
        child: KeepClear(child: _probe()),
      );

      const bottom = EdgeInsets.only(bottom: 34);
      expect(_inside.padding, bottom);
      expect(_inside.viewPadding, bottom);
      expect(tester.getTopLeft(find.byKey(_content)), const Offset(47, 59));
    });

    testWidgets('leaves the sides it is not asked to keep clear of', (
      tester,
    ) async {
      const insets = EdgeInsets.fromLTRB(47, 59, 47, 34);
      await _pump(
        tester,
        size: _phone.flipped,
        viewPadding: insets,
        child: KeepClear(top: false, right: false, child: _probe()),
      );

      const left = EdgeInsets.fromLTRB(0, 59, 47, 34);
      expect(_inside.padding, left);
      expect(_inside.viewPadding, left);
      expect(tester.getTopLeft(find.byKey(_content)), const Offset(47, 0));
    });
  });

  testWidgets('the select-library list has no room above or below it', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = _phone;
    tester.view.viewPadding = _fake(_portrait);
    tester.view.padding = _fake(_portrait);
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
          ChangeNotifierProvider<LibraryPathProvider>.value(
            value: _Libraries(),
          ),
          ChangeNotifierProvider<LibraryManagerProvider>.value(
            value: _LibraryManager(),
          ),
        ],
        child: const FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: HomePage(),
        ),
      ),
    );
    await tester.pump(const Duration(seconds: 2));

    final list = tester.getRect(find.byType(ListView));
    expect(tester.getRect(find.byType(AddLibrarySettingButton)).top, list.top);
    expect(
      tester.getRect(find.byType(SettingsButton).last).bottom,
      list.bottom,
    );
  });

  testWidgets('the car disk keeps clear of a notch at the right', (
    tester,
  ) async {
    await _pump(
      tester,
      size: const Size(800, 320),
      viewPadding: _landscape,
      // The disk slides in from its first place, which is measured.
      settle: false,
      child: ChangeNotifierProvider<PlaybackStatusProvider>.value(
        value: _Status(),
        child: const Stack(children: [CoverArtDisk()]),
      ),
    );

    final disk = tester.widget<Positioned>(
      find.descendant(
        of: find.byType(CoverArtDisk),
        matching: find.byType(Positioned),
      ),
    );
    expect(disk.right, _landscape.right);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(seconds: 2));
  });

  group('StatusBarStyle', () {
    SystemUiOverlayStyle? styleOf(WidgetTester tester) => tester
        .widget<AnnotatedRegion<SystemUiOverlayStyle>>(
          find.byType(AnnotatedRegion<SystemUiOverlayStyle>),
        )
        .value;

    for (final brightness in Brightness.values) {
      testWidgets('follows the $brightness theme', (tester) async {
        await tester.pumpWidget(
          FluentTheme(
            data: FluentThemeData(brightness: brightness),
            child: const StatusBarStyle(child: SizedBox()),
          ),
        );

        final style = styleOf(tester)!;
        final dark = brightness == Brightness.dark;
        // iOS reads the bar's background, Android the icons' brightness.
        expect(
          style.statusBarBrightness,
          dark ? Brightness.dark : Brightness.light,
        );
        expect(
          style.statusBarIconBrightness,
          dark ? Brightness.light : Brightness.dark,
        );
        // The navigation bar of Android is left alone.
        expect(style.systemNavigationBarColor, isNull);
        expect(style.systemNavigationBarIconBrightness, isNull);
      });
    }

    testWidgets('a page on a dark background in the light theme says so', (
      tester,
    ) async {
      await tester.pumpWidget(
        FluentTheme(
          data: FluentThemeData(brightness: Brightness.light),
          child: const StatusBarStyle(
            brightness: Brightness.dark,
            child: SizedBox(),
          ),
        ),
      );

      expect(styleOf(tester)!.statusBarIconBrightness, Brightness.light);
    });

    testWidgets('a dark page under the status bar wins over the app theme', (
      tester,
    ) async {
      await tester.pumpWidget(
        FluentTheme(
          data: FluentThemeData(brightness: Brightness.light),
          child: StatusBarStyle(
            child: FluentTheme(
              data: FluentThemeData(brightness: Brightness.dark),
              child: const StatusBarStyle(child: SizedBox.expand()),
            ),
          ),
        ),
      );

      // The binding picks the style the same way, at the top of the view.
      final layer = tester.binding.renderViews.first.debugLayer!;
      final style = layer.find<SystemUiOverlayStyle>(const Offset(100, 1));
      expect(style?.statusBarBrightness, Brightness.dark);
    });
  });
}

class _Libraries extends ChangeNotifier implements LibraryPathProvider {
  final _entries = [LibraryPathEntry('/music'), LibraryPathEntry('/podcasts')];

  @override
  Set<LibraryPathEntry> get libraryHistory => _entries.toSet();

  @override
  List<LibraryPathEntry> getLLPaths() => _entries;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _LibraryManager extends ChangeNotifier implements LibraryManagerProvider {
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _Status extends ChangeNotifier implements PlaybackStatusProvider {
  @override
  final playbackStatus = PlaybackStatusState();

  @override
  bool get notReady => false;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
