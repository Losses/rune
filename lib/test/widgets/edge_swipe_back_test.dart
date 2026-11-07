import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/utils/platform.dart';
import 'package:rune/widgets/router/edge_swipe_back.dart';

const _phone = Size(400, 800);

// A phone on its side, with a notch at the left.
const _side = Size(844, 390);
const _notch = EdgeInsets.only(left: 59, right: 59, bottom: 21);

// The swipe is iOS's; the tests run as iOS unless they say otherwise.
final _iOS = TargetPlatformVariant.only(TargetPlatform.iOS);

// A window of [size], with Rune's providers for its layout.
Future<void> _pump(
  WidgetTester tester,
  Widget child, {
  Size size = _phone,
  EdgeInsets viewPadding = EdgeInsets.zero,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = size;
  final padding = FakeViewPadding(
    left: viewPadding.left,
    top: viewPadding.top,
    right: viewPadding.right,
    bottom: viewPadding.bottom,
  );
  tester.view.viewPadding = padding;
  tester.view.padding = padding;
  addTearDown(tester.view.reset);
  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider(
          create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
        ),
        ChangeNotifierProvider(
          create: (context) => ResponsiveProvider(
            context.read<ScreenSizeProvider>(),
            () async => null,
          ),
        ),
      ],
      child: Directionality(textDirection: TextDirection.ltr, child: child),
    ),
  );
}

void main() {
  Future<int> drag(
    WidgetTester tester, {
    required Offset from,
    required Offset by,
    Size size = _phone,
    bool enabled = true,
    PointerDeviceKind kind = PointerDeviceKind.touch,
  }) async {
    var calls = 0;
    await _pump(
      tester,
      EdgeSwipeBack(
        enabled: enabled,
        onBack: () => calls++,
        child: const SizedBox.expand(),
      ),
      size: size,
    );
    await tester.dragFrom(from, by, kind: kind);
    await tester.pumpAndSettle();
    return calls;
  }

  testWidgets('a drag from the edge goes back once', (tester) async {
    expect(
      await drag(tester, from: const Offset(5, 400), by: const Offset(145, 0)),
      1,
    );
  }, variant: _iOS);

  testWidgets('a drag that starts away from the edge does not', (tester) async {
    expect(
      await drag(
        tester,
        from: const Offset(100, 400),
        by: const Offset(100, 0),
      ),
      0,
    );
  }, variant: _iOS);

  testWidgets('a short drag does not', (tester) async {
    expect(
      await drag(tester, from: const Offset(5, 400), by: const Offset(20, 0)),
      0,
    );
  }, variant: _iOS);

  testWidgets('a leftward drag does not', (tester) async {
    expect(
      await drag(
        tester,
        from: const Offset(15, 400),
        by: const Offset(-100, 0),
      ),
      0,
    );
  }, variant: _iOS);

  testWidgets('a drag in the header area does not', (tester) async {
    expect(
      await drag(tester, from: const Offset(5, 50), by: const Offset(145, 0)),
      0,
    );
  }, variant: _iOS);

  testWidgets('the strip starts below the header in the car layout', (
    tester,
  ) async {
    // The car layout's header is lower than the full one.
    const car = Size(800, 320);
    expect(
      await drag(
        tester,
        from: const Offset(5, 90),
        by: const Offset(145, 0),
        size: car,
      ),
      1,
    );
    expect(
      await drag(
        tester,
        from: const Offset(5, 70),
        by: const Offset(145, 0),
        size: car,
      ),
      0,
    );
  }, variant: _iOS);

  testWidgets('disabled never goes back', (tester) async {
    expect(
      await drag(
        tester,
        from: const Offset(5, 400),
        by: const Offset(145, 0),
        enabled: false,
      ),
      0,
    );
  }, variant: _iOS);

  group('the page under the strip', () {
    // A vertical list, as on a real page.
    Future<ScrollController> pumpPage(WidgetTester tester) async {
      final scroll = ScrollController();
      addTearDown(scroll.dispose);
      await _pump(
        tester,
        EdgeSwipeBack(
          onBack: () {},
          child: ListView.builder(
            controller: scroll,
            itemExtent: 100,
            itemCount: 50,
            itemBuilder: (context, index) => const SizedBox.expand(),
          ),
        ),
      );
      return scroll;
    }

    testWidgets('still gets a tap that lands on the strip', (tester) async {
      var taps = 0;
      await _pump(
        tester,
        EdgeSwipeBack(
          onBack: () {},
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () => taps++,
            child: const SizedBox.expand(),
          ),
        ),
      );
      await tester.tapAt(const Offset(5, 400));
      await tester.pumpAndSettle();
      expect(taps, 1);
    }, variant: _iOS);

    testWidgets('still scrolls from a drag that starts on the strip', (
      tester,
    ) async {
      final scroll = await pumpPage(tester);
      await tester.dragFrom(const Offset(5, 400), const Offset(0, -200));
      await tester.pumpAndSettle();
      expect(scroll.offset, greaterThan(0));
    }, variant: _iOS);

    testWidgets('a swipe over a scrolling page still goes back', (
      tester,
    ) async {
      var backs = 0;
      await _pump(
        tester,
        EdgeSwipeBack(
          onBack: () => backs++,
          child: ListView(
            children: [
              for (var i = 0; i < 50; i++) const SizedBox(height: 100),
            ],
          ),
        ),
      );
      await tester.dragFrom(const Offset(5, 400), const Offset(145, 0));
      await tester.pumpAndSettle();
      expect(backs, 1);
    }, variant: _iOS);

    group('a horizontal list, on a phone on its side', () {
      // The track grids scroll across under the strip, which is as wide as
      // the notch.
      Future<(ScrollController, ValueGetter<int>)> pumpList(
        WidgetTester tester,
      ) async {
        var backs = 0;
        final scroll = ScrollController();
        addTearDown(scroll.dispose);
        await _pump(
          tester,
          EdgeSwipeBack(
            onBack: () => backs++,
            child: ListView.builder(
              controller: scroll,
              scrollDirection: Axis.horizontal,
              itemExtent: 100,
              itemCount: 50,
              itemBuilder: (context, index) => const SizedBox.expand(),
            ),
          ),
          size: _side,
          viewPadding: _notch,
        );
        return (scroll, () => backs);
      }

      testWidgets('scrolls from a leftward drag on the strip, which does not '
          'go back', (tester) async {
        final (scroll, backs) = await pumpList(tester);
        await tester.dragFrom(const Offset(50, 250), const Offset(-45, 0));
        await tester.pumpAndSettle();
        expect(scroll.offset, greaterThan(0));
        expect(backs(), 0);
      }, variant: _iOS);

      testWidgets('goes back from a rightward drag on the strip', (
        tester,
      ) async {
        final (scroll, backs) = await pumpList(tester);
        await tester.dragFrom(const Offset(50, 250), const Offset(145, 0));
        await tester.pumpAndSettle();
        expect(backs(), 1);
        expect(scroll.offset, 0);
      }, variant: _iOS);
    });
  });

  testWidgets(
    'never goes back elsewhere',
    (tester) async {
      expect(
        await drag(
          tester,
          from: const Offset(5, 400),
          by: const Offset(145, 0),
        ),
        0,
      );
    },
    variant: TargetPlatformVariant({
      TargetPlatform.android,
      TargetPlatform.macOS,
    }),
  );

  testWidgets('a mouse drag from the edge does not go back', (tester) async {
    expect(
      await drag(
        tester,
        from: const Offset(5, 400),
        by: const Offset(145, 0),
        kind: PointerDeviceKind.mouse,
      ),
      0,
    );
  }, variant: _iOS);

  testWidgets('the iOS build on a Mac has no strip', (tester) async {
    debugIosDevice = IosDevice.mac;
    addTearDown(() => debugIosDevice = null);
    expect(
      await drag(tester, from: const Offset(5, 400), by: const Offset(145, 0)),
      0,
    );
    expect(
      find.descendant(
        of: find.byType(EdgeSwipeBack),
        matching: find.byType(RawGestureDetector),
      ),
      findsNothing,
    );
  }, variant: _iOS);
}
