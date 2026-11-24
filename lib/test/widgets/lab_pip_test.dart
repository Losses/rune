import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/utils/ax_shadow.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/responsive_providers.dart';
import 'package:rune/providers/visualizer_lab.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_overlay.dart';
import 'package:rune/widgets/visualizer/lab/lab_pip.dart';
import 'package:rune/widgets/visualizer/lab/lab_pip_layout.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';
import 'package:rune/widgets/visualizer/visualizer_controller.dart';

class _FakeLab extends ChangeNotifier implements VisualizerLabProvider {
  _FakeLab(this.pipLayout);

  @override
  final now = ValueNotifier<VisualizerLabNow?>(null);

  @override
  final timeline = ValueNotifier<LabTimelineData>(LabTimelineData(length: 180));

  @override
  final rowsModel = LabRowsModel();

  @override
  bool peeking = false;

  final peeks = <bool>[];

  @override
  double? cardsHeight;

  @override
  double? sceneWidth;

  @override
  double? get labelWidth => null;

  @override
  LabPipLayout pipLayout;

  final saved = <LabPipLayout>[];

  @override
  void setPipLayout(LabPipLayout layout, {bool save = true}) {
    pipLayout = layout;
    if (save) saved.add(layout);
    notifyListeners();
  }

  @override
  void setBodyShown(bool shown) {}

  @override
  void setPeeking(bool value) {
    peeks.add(value);
    peeking = value;
    notifyListeners();
  }

  @override
  void play(VisualizerLabCommand command, {String? markLabel}) {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _FakeScenes extends ChangeNotifier implements VisualizerScenesProvider {
  @override
  List<VisualizerScene> get scenes => const [];

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

// The picture keeps 12 inside the lab's padding: on a phone upright that is
// 8 (10 at the bottom), so it rests 20 in (22 from the bottom); on its side 14
// across, 10 at the top and 12 at the bottom.
const _up = Size(390, 844);
const _side = Size(844, 390);

void main() {
  late _FakeLab lab;

  Future<void> pump(
    WidgetTester tester,
    Size size, {
    LabPipLayout layout = const LabPipLayout(),
  }) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    lab = _FakeLab(layout);
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<VisualizerLabProvider>.value(value: lab),
          ChangeNotifierProvider<VisualizerScenesProvider>.value(
            value: _FakeScenes(),
          ),
          ChangeNotifierProvider(
            create: (_) => ScreenSizeProvider.withWriter(null, (_) async {}),
          ),
        ],
        child: FluentApp(home: LabBody(controller: VisualizerController())),
      ),
    );
    await tester.pump();
  }

  Finder pip() =>
      find.descendant(of: find.byType(LabPip), matching: find.byType(Card));
  Rect rect(WidgetTester tester) => tester.getRect(pip());

  // A press, a move by [by] at [speed] pixels a second, and a release, with
  // the time of each event set so that the release has that speed.
  Future<void> fling(
    WidgetTester tester,
    PointerDeviceKind kind,
    Offset from,
    Offset by,
    double speed,
  ) async {
    final gesture = await tester.createGesture(kind: kind);
    await gesture.down(from);
    const steps = 10;
    final step = Duration(
      microseconds: (by.distance / speed * 1e6 / steps).round(),
    );
    for (var i = 1; i <= steps; i++) {
      await gesture.moveTo(from + by * (i / steps), timeStamp: step * i);
    }
    await gesture.up(timeStamp: step * steps);
    await tester.pumpAndSettle();
  }

  // A drag that comes to rest before it lets go.
  Future<void> drag(WidgetTester tester, Offset from, Offset by) async {
    final gesture = await tester.startGesture(from);
    for (var i = 0; i < 10; i++) {
      await gesture.moveBy(by / 10);
      await tester.pump(const Duration(milliseconds: 16));
    }
    await gesture.up();
    await tester.pumpAndSettle();
  }

  const kinds = {
    'a finger': PointerDeviceKind.touch,
    'the mouse': PointerDeviceKind.mouse,
  };

  for (final entry in kinds.entries) {
    final kind = entry.value;
    final who = entry.key;

    testWidgets('$who: a flick toward a corner settles there from far away, '
        'a slow release in the nearest', (tester) async {
      await pump(tester, _up);
      // Short, slanted and fast: toward the top left.
      await fling(
        tester,
        kind,
        rect(tester).center,
        const Offset(-80, -140),
        2500,
      );
      var at = rect(tester);
      expect(at.left, closeTo(20, 0.5));
      expect(at.top, closeTo(20, 0.5));
      expect(lab.saved.last.corner, Alignment.topLeft);
      expect(lab.saved.last.stash, LabPipSide.none);

      // Dragged slowly a little to the right: still the top left.
      await fling(tester, kind, rect(tester).center, const Offset(60, 20), 10);
      expect(rect(tester).left, closeTo(20, 0.5));
      // Slowly to the lower half and the right half.
      await fling(
        tester,
        kind,
        rect(tester).center,
        const Offset(200, 500),
        10,
      );
      at = rect(tester);
      expect(at.right, closeTo(390 - 20, 0.5));
      expect(at.bottom, closeTo(844 - 22, 0.5));
    });

    testWidgets('$who: a flick past the right edge tucks it into the side, '
        'a tap or a drag brings it out', (tester) async {
      await pump(tester, _up);
      await fling(tester, kind, rect(tester).center, const Offset(40, 0), 2500);
      var at = rect(tester);
      expect(at.left, closeTo(390 - 28, 0.5));
      expect(lab.saved.last.stash, LabPipSide.right);
      expect(find.byIcon(Symbols.chevron_left), findsOneWidget);
      final y = at.top;

      // Tucked it does not peek, however long it is held.
      final hold = await tester.createGesture(kind: kind);
      await hold.down(Offset(390 - 14, at.center.dy));
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 100));
      expect(lab.peeks, isEmpty);
      await hold.up();
      await tester.pumpAndSettle();
      // That was a tap: out into a right-hand corner at that height.
      at = rect(tester);
      expect(at.right, closeTo(390 - 20, 0.5));
      expect(lab.saved.last.stash, LabPipSide.none);
      expect(at.top, greaterThan(y - 1));

      // Tucked again at the left, and dragged back in.
      await fling(tester, kind, at.center, const Offset(-300, 0), 3000);
      at = rect(tester);
      expect(at.right, closeTo(28, 0.5));
      expect(lab.saved.last.stash, LabPipSide.left);
      expect(find.byIcon(Symbols.chevron_right), findsOneWidget);
      final back = await tester.createGesture(kind: kind);
      await back.down(Offset(14, at.center.dy));
      for (var i = 0; i < 10; i++) {
        await back.moveBy(const Offset(20, 0));
        await tester.pump(const Duration(milliseconds: 16));
      }
      await back.up();
      await tester.pumpAndSettle();
      at = rect(tester);
      expect(at.left, closeTo(20, 0.5));
      expect(lab.saved.last.stash, LabPipSide.none);
    });

    testWidgets('$who: it can be dragged most of the way off a side and stays '
        'tucked when let go slowly', (tester) async {
      await pump(tester, _up);
      await drag(tester, rect(tester).center, const Offset(-400, -100));
      expect(rect(tester).right, closeTo(28, 0.5));
      expect(lab.saved.last.stash, LabPipSide.left);
    });
  }

  testWidgets('a pinch of two fingers grows it, bounded, from its corner', (
    tester,
  ) async {
    await pump(tester, _up);
    final start = rect(tester);
    final a = await tester.startGesture(
      start.center - const Offset(20, 0),
      pointer: 1,
    );
    final b = await tester.startGesture(
      start.center + const Offset(20, 0),
      pointer: 2,
    );
    await a.moveBy(const Offset(-20, 0));
    await b.moveBy(const Offset(20, 0));
    await tester.pump();
    var at = rect(tester);
    expect(at.width, greaterThan(156 * 1.5));
    expect(at.right, closeTo(390 - 20, 0.5));
    expect(at.bottom, closeTo(844 - 22, 0.5));
    // Held past the most.
    await a.moveBy(const Offset(-300, 0));
    await b.moveBy(const Offset(300, 0));
    await tester.pump();
    expect(rect(tester).width, closeTo(390 - 2 * 20, 0.5));
    await a.up();
    await b.up();
    await tester.pumpAndSettle();
    at = rect(tester);
    expect(at.width, closeTo(350, 0.5));
    expect(at.right, closeTo(390 - 20, 0.5));
    expect(lab.saved.last.width, closeTo(350, 0.5));
    expect(lab.saved.last.corner, Alignment.bottomRight);
    expect(lab.peeks, isEmpty);

    // And smaller, down to the least.
    final c = await tester.startGesture(at.center - const Offset(60, 0));
    final d = await tester.startGesture(at.center + const Offset(60, 0));
    await c.moveBy(const Offset(55, 0));
    await d.moveBy(const Offset(-55, 0));
    await tester.pump();
    expect(rect(tester).width, closeTo(120, 0.5));
    await c.up();
    await d.up();
    await tester.pumpAndSettle();
  });

  testWidgets('a second finger during a drag makes it a pinch', (tester) async {
    await pump(tester, _up);
    final start = rect(tester);
    final a = await tester.startGesture(start.center, pointer: 1);
    await a.moveBy(const Offset(-100, -200));
    await tester.pump();
    final moved = rect(tester);
    final b = await tester.startGesture(
      moved.center + const Offset(30, 0),
      pointer: 2,
    );
    await b.moveBy(const Offset(40, 0));
    await tester.pump();
    final grown = rect(tester);
    expect(grown.width, greaterThan(moved.width));
    // Scaled from the corner of the box nearest to where it was dragged:
    // the lower right here.
    expect(grown.right, closeTo(moved.right, 0.5));
    await a.up();
    await b.up();
    await tester.pumpAndSettle();
    expect(lab.saved, isNotEmpty);
  });

  testWidgets('a double tap or click switches between default and most '
      'width', (tester) async {
    await pump(tester, _up);
    expect(rect(tester).width, closeTo(156, 0.5));
    await tester.tapAt(rect(tester).center);
    await tester.pump(const Duration(milliseconds: 80));
    await tester.tapAt(rect(tester).center);
    await tester.pump(const Duration(milliseconds: 100));
    // On its way, animated.
    await tester.pump(const Duration(milliseconds: 100));
    expect(rect(tester).width, inExclusiveRange(156, 350));
    await tester.pumpAndSettle();
    expect(rect(tester).width, closeTo(350, 0.5));
    expect(rect(tester).right, closeTo(390 - 20, 0.5));
    expect(lab.pipLayout.width, closeTo(350, 0.5));

    // With the mouse.
    for (var i = 0; i < 2; i++) {
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.down(rect(tester).center);
      await mouse.up();
      await tester.pump(const Duration(milliseconds: 80));
    }
    await tester.pumpAndSettle();
    expect(rect(tester).width, closeTo(156, 0.5));
    expect(lab.pipLayout.width, isNull);

    // Two taps far apart in time are two taps.
    await tester.tapAt(rect(tester).center);
    await tester.pump(const Duration(seconds: 1));
    await tester.tapAt(rect(tester).center);
    await tester.pumpAndSettle();
    expect(rect(tester).width, closeTo(156, 0.5));
  });

  testWidgets('it floats over the pages with the app\'s shadow, and keeps '
      'room inside them at its most', (tester) async {
    await pump(tester, _up);
    final shadow = tester.widget<DecoratedBox>(
      find.ancestor(of: pip(), matching: find.byType(DecoratedBox)).first,
    );
    expect((shadow.decoration as BoxDecoration).boxShadow, axShadow(20));
    await tester.tapAt(rect(tester).center);
    await tester.pump(const Duration(milliseconds: 80));
    await tester.tapAt(rect(tester).center);
    await tester.pumpAndSettle();
    // 12 inside the pages' padding of 8 on either side.
    expect(rect(tester).left, closeTo(20, 0.5));
    expect(rect(tester).right, closeTo(390 - 20, 0.5));
  });

  testWidgets('on its side its most fills the height inside the margins, the '
      'width following the shape', (tester) async {
    await pump(tester, _side);
    await tester.tapAt(rect(tester).center);
    await tester.pump(const Duration(milliseconds: 80));
    await tester.tapAt(rect(tester).center);
    await tester.pumpAndSettle();
    final most = rect(tester);
    expect(most.height, closeTo(390 - (10 + 12) - (12 + 12), 0.5));
    expect(most.width, closeTo((390 - (10 + 12) - (12 + 12)) * 16 / 9, 0.5));
    expect(most.bottomRight, const Offset(844 - 26, 390 - 24));
  });

  testWidgets('Ctrl or Cmd and the mouse wheel over it scale it, the wheel '
      'alone does not', (tester) async {
    await pump(tester, _up);
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(mouse.hover(rect(tester).center));
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, -100)));
    await tester.pump(const Duration(seconds: 1));
    expect(rect(tester).width, closeTo(156, 0.5));

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, -100)));
    await tester.pump();
    expect(rect(tester).width, greaterThan(170));
    expect(rect(tester).right, closeTo(390 - 20, 0.5));
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, -1000)));
    await tester.pump(const Duration(seconds: 1));
    expect(rect(tester).width, closeTo(350, 0.5));
    expect(lab.saved.last.width, closeTo(350, 0.5));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);

    await tester.sendKeyDownEvent(LogicalKeyboardKey.metaLeft);
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, 1000)));
    await tester.pump(const Duration(seconds: 1));
    expect(rect(tester).width, closeTo(120, 0.5));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.metaLeft);
  });

  testWidgets('a tap on a tucked picture\'s tab springs it out, it is not at '
      'its corner at once', (tester) async {
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(stash: LabPipSide.right),
    );
    final tab = rect(tester);
    expect(tab.left, closeTo(390 - 28, 0.5));
    await tester.tapAt(Offset(390 - 14, tab.center.dy));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 60));
    final left = rect(tester).left;
    expect(left, lessThan(390 - 28 - 1));
    expect(left, greaterThan(390 - 20 - 156 + 1));
    await tester.pumpAndSettle();
    expect(rect(tester).right, closeTo(390 - 20, 0.5));
  });

  testWidgets('a press soon after a wheel resize ends the resize: the width '
      'is saved and a double tap still changes it', (tester) async {
    await pump(tester, _up);
    final mouse = TestPointer(1, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(mouse.hover(rect(tester).center));
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, -100)));
    await tester.pump(const Duration(milliseconds: 100));
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    final grown = rect(tester).width;
    expect(grown, greaterThan(170));
    expect(lab.saved, isEmpty);

    Future<void> click() async {
      final press = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await press.down(rect(tester).center);
      await press.up();
      await tester.pump(const Duration(milliseconds: 60));
    }

    await click();
    expect(lab.saved.last.width, closeTo(grown, 0.5));
    await click();
    await tester.pumpAndSettle();
    expect(rect(tester).width, closeTo(350, 0.5));
  });

  testWidgets('a wide tucked picture is dragged from where it rests, with no '
      'jump', (tester) async {
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(stash: LabPipSide.right, width: 350),
    );
    final y = rect(tester).center.dy;
    expect(rect(tester).left, closeTo(390 - 28, 0.5));
    final finger = await tester.startGesture(Offset(390 - 14, y));
    await finger.moveBy(const Offset(-30, 0));
    await tester.pump();
    final first = rect(tester).left;
    await finger.moveBy(const Offset(-10, 0));
    await tester.pump();
    expect(rect(tester).left, closeTo(first - 10, 0.5));
    // Where it was when the drag began, not moved in.
    expect(first, closeTo(390 - 28, 0.5));
    await finger.up();
    await tester.pumpAndSettle();
  });

  testWidgets('the tab keeps clear of the screen\'s side safe area', (
    tester,
  ) async {
    tester.view.viewPadding = const FakeViewPadding(left: 44, right: 44);
    addTearDown(tester.view.resetViewPadding);
    await pump(
      tester,
      _side,
      layout: const LabPipLayout(stash: LabPipSide.right),
    );
    expect(rect(tester).left, closeTo(844 - 44 - 28, 0.5));
    await pump(
      tester,
      _side,
      layout: const LabPipLayout(stash: LabPipSide.left),
    );
    expect(rect(tester).right, closeTo(44 + 28, 0.5));
  });

  testWidgets('the mouse resizes it by the corner away from the one it rests '
      'in, and drags it by the rest', (tester) async {
    await pump(tester, _up);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.addPointer(location: Offset.zero);
    MouseCursor cursor() =>
        RendererBinding.instance.mouseTracker.debugDeviceActiveCursor(1)!;

    final start = rect(tester);
    await mouse.moveTo(start.topLeft + const Offset(4, 4));
    await tester.pump();
    expect(cursor(), SystemMouseCursors.resizeUpLeftDownRight);
    await mouse.moveTo(start.center);
    await tester.pump();
    expect(cursor(), SystemMouseCursors.grab);
    // Not the corner it rests in.
    await mouse.moveTo(start.bottomRight - const Offset(4, 4));
    await tester.pump();
    expect(cursor(), SystemMouseCursors.grab);

    await mouse.moveTo(start.topLeft + const Offset(4, 4));
    await mouse.down(start.topLeft + const Offset(4, 4));
    await mouse.moveBy(const Offset(-30, -30));
    await mouse.moveBy(const Offset(-30, -30));
    await tester.pump();
    await tester.pump();
    expect(cursor(), SystemMouseCursors.resizeUpLeftDownRight);
    final grown = rect(tester);
    expect(grown.width, greaterThan(start.width + 20));
    expect(grown.right, closeTo(start.right, 0.5));
    expect(grown.bottom, closeTo(start.bottom, 0.5));
    await mouse.moveBy(const Offset(-400, -400));
    await tester.pump();
    expect(rect(tester).width, closeTo(350, 0.5));
    await mouse.moveBy(const Offset(800, 800));
    await tester.pump();
    expect(rect(tester).width, closeTo(120, 0.5));
    await mouse.up();
    await tester.pumpAndSettle();
    expect(rect(tester).right, closeTo(390 - 20, 0.5));
    expect(lab.saved.last.width, closeTo(120, 0.5));
    expect(lab.peeks, isEmpty);

    // From a corner on the other diagonal.
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(corner: Alignment.topRight),
    );
    await mouse.moveTo(rect(tester).bottomLeft + const Offset(4, -4));
    await tester.pump();
    expect(cursor(), SystemMouseCursors.resizeUpRightDownLeft);
  });

  testWidgets('a two-finger slide on a trackpad moves it and flings it, a '
      'pinch scales it', (tester) async {
    await pump(tester, _up);
    final pad = TestPointer(2, PointerDeviceKind.trackpad);
    final centre = rect(tester).center;
    await tester.sendEventToBinding(pad.panZoomStart(centre));
    for (var i = 1; i <= 10; i++) {
      await tester.sendEventToBinding(
        pad.panZoomUpdate(
          centre,
          pan: Offset(-8, -14) * i.toDouble(),
          timeStamp: Duration(milliseconds: i * 4),
        ),
      );
    }
    await tester.pump();
    // Followed the slide.
    expect(rect(tester).left, lessThan(centre.dx - 78 - 50));
    await tester.sendEventToBinding(
      pad.panZoomEnd(timeStamp: const Duration(milliseconds: 40)),
    );
    await tester.pumpAndSettle();
    expect(lab.saved.last.corner, Alignment.topLeft);
    expect(rect(tester).topLeft, const Offset(20, 20));

    // A pinch.
    final start = rect(tester);
    await tester.sendEventToBinding(pad.panZoomStart(start.center));
    await tester.sendEventToBinding(
      pad.panZoomUpdate(start.center, scale: 1.3),
    );
    await tester.pump();
    expect(rect(tester).width, closeTo(156 * 1.3, 1));
    expect(rect(tester).topLeft, const Offset(20, 20));
    await tester.sendEventToBinding(pad.panZoomUpdate(start.center, scale: 5));
    await tester.pump();
    expect(rect(tester).width, closeTo(350, 0.5));
    await tester.sendEventToBinding(pad.panZoomEnd());
    await tester.pumpAndSettle();
    expect(lab.saved.last.width, closeTo(350, 0.5));
  });

  testWidgets('a trackpad slide or pinch over it is taken by it and does not '
      'reach what is under, one beside it does', (tester) async {
    tester.view.physicalSize = _up;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    var under = 0;
    await tester.pumpWidget(
      FluentApp(
        home: Stack(
          children: [
            Positioned.fill(
              child: GestureDetector(
                onScaleUpdate: (_) => under++,
                child: const ColoredBox(color: Color(0xFF000000)),
              ),
            ),
            LabPip(
              controller: VisualizerController(),
              area: _up,
              insets: EdgeInsets.zero,
              layout: const LabPipLayout(),
              onLayout: (_) {},
              onHold: (_) {},
            ),
          ],
        ),
      ),
    );
    final pad = TestPointer(4, PointerDeviceKind.trackpad);
    Future<void> slide(Offset at) async {
      await tester.sendEventToBinding(pad.panZoomStart(at));
      for (var i = 1; i <= 4; i++) {
        await tester.sendEventToBinding(
          pad.panZoomUpdate(at, pan: Offset(-20, -20) * i.toDouble()),
        );
      }
      await tester.sendEventToBinding(pad.panZoomEnd());
      await tester.pumpAndSettle();
    }

    await slide(tester.getCenter(pip()));
    expect(under, 0);
    await slide(const Offset(100, 200));
    expect(under, greaterThan(0));
  });

  testWidgets('long press with a held mouse button peeks, a move first does '
      'not', (tester) async {
    await pump(tester, _up);
    final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
    await mouse.down(rect(tester).center);
    await tester.pump(kLongPressTimeout + const Duration(milliseconds: 50));
    expect(lab.peeks, [true]);
    await mouse.up();
    await tester.pump();
    expect(lab.peeks, [true, false]);
  });

  testWidgets('what is saved comes back: the picture starts in its corner, '
      'its width and its side', (tester) async {
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(corner: Alignment.topLeft, width: 200),
    );
    var at = rect(tester);
    expect(at.topLeft, const Offset(20, 20));
    expect(at.width, closeTo(200, 0.5));

    await pump(
      tester,
      _up,
      layout: const LabPipLayout(
        corner: Alignment.topLeft,
        stash: LabPipSide.right,
        stashAt: 1,
      ),
    );
    at = rect(tester);
    expect(at.left, closeTo(390 - 28, 0.5));
    expect(at.bottom, closeTo(844 - 22, 0.5));
    final semantics = tester.ensureSemantics();
    expect(find.bySemanticsLabel('Show Scene Picture'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('the provider gets the corner, the width and the side on '
      'release only', (tester) async {
    await pump(tester, _up);
    final gesture = await tester.startGesture(rect(tester).center);
    for (var i = 0; i < 9; i++) {
      await gesture.moveBy(const Offset(-30, -70));
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(lab.saved, isEmpty);
    await gesture.up();
    await tester.pumpAndSettle();
    expect(lab.saved, hasLength(1));
    expect(lab.saved.single.corner, Alignment.topLeft);
    expect(lab.saved.single.width, isNull);
    expect(lab.saved.single.stash, LabPipSide.none);
  });

  testWidgets('turning the window keeps the corner and clamps the width', (
    tester,
  ) async {
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(corner: Alignment.topRight, width: 250),
    );
    expect(rect(tester).width, closeTo(250, 0.5));
    expect(rect(tester).topRight, const Offset(390 - 20, 20));
    tester.view.physicalSize = _side;
    await tester.pump();
    expect(rect(tester).topRight, const Offset(844 - 26, 22));
    // A narrower window allows less: its width inside the margins.
    tester.view.physicalSize = const Size(220, 600);
    await tester.pump();
    expect(rect(tester).width, closeTo(220 - 2 * 20, 0.5));
    expect(rect(tester).topRight, const Offset(220 - 20, 20));
  });

  testWidgets('a tucked picture stays in its side when the window turns', (
    tester,
  ) async {
    await pump(
      tester,
      _up,
      layout: const LabPipLayout(stash: LabPipSide.right, stashAt: 0),
    );
    tester.view.physicalSize = _side;
    await tester.pump();
    expect(rect(tester).left, closeTo(844 - 28, 0.5));
    expect(rect(tester).top, closeTo(22, 0.5));
  });
}
