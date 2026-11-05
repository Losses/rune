import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_lab.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/widgets/visualizer/lab/lab_cards.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_overlay.dart';
import 'package:rune/widgets/visualizer/lab/lab_pip_layout.dart';
import 'package:rune/widgets/visualizer/lab/lab_pages.dart';
import 'package:rune/widgets/visualizer/lab/lab_pip.dart';
import 'package:rune/widgets/visualizer/lab/lab_splitter.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';
import 'package:rune/widgets/visualizer/visualizer_controller.dart';

class _FakeLab extends ChangeNotifier implements VisualizerLabProvider {
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
  LabPipLayout pipLayout = const LabPipLayout();

  @override
  void setPipLayout(LabPipLayout layout, {bool save = true}) {
    pipLayout = layout;
    notifyListeners();
  }

  @override
  double? get labelWidth => null;

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

const _phoneUp = Size(390, 844);
const _phoneSide = Size(844, 390);

void main() {
  Future<_FakeLab> pump(WidgetTester tester, Size size) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final lab = _FakeLab();
    await tester.pumpWidget(
      MultiProvider(
        providers: [
          ChangeNotifierProvider<VisualizerLabProvider>.value(value: lab),
          ChangeNotifierProvider<VisualizerScenesProvider>.value(
            value: _FakeScenes(),
          ),
        ],
        child: FluentApp(home: LabBody(controller: VisualizerController())),
      ),
    );
    await tester.pump();
    return lab;
  }

  Finder card(String label) =>
      find.byWidgetPredicate((w) => w is LabCard && w.label == label);
  // The handle on [page], which is named by the page it shows.
  Finder handleOn(String page) => find.byWidgetPredicate(
    (w) =>
        w is LabPageHandle &&
        w.label == (page == 'Timeline' ? 'Cards' : 'Timeline'),
  );
  Finder pip() =>
      find.descendant(of: find.byType(LabPip), matching: find.byType(Card));

  Future<void> dragHandle(WidgetTester tester, Finder from, Offset by) async {
    final gesture = await tester.startGesture(tester.getCenter(from));
    for (var i = 0; i < 10; i++) {
      await gesture.moveBy(by / 10);
      await tester.pump(const Duration(milliseconds: 20));
    }
    // Rests before it lifts, so there is no fling.
    await tester.pump(const Duration(milliseconds: 300));
    await gesture.up();
    await tester.pumpAndSettle();
  }

  bool onCardsPage(WidgetTester tester) =>
      (Offset.zero & tester.view.physicalSize).contains(
        tester.getCenter(card('Play')),
      );

  for (final size in [_phoneUp, _phoneSide]) {
    final down = size == _phoneUp;
    final name = down ? 'upright' : 'on its side';

    testWidgets('a phone $name shows the timeline, with the cards off screen '
        'and a floating picture', (tester) async {
      await pump(tester, size);
      expect(tester.takeException(), isNull);
      expect(find.byType(LabSplitter), findsNothing);
      expect(card('Scene'), findsNothing);
      expect(find.byType(LabPip), findsOneWidget);
      final timeline = tester.getRect(find.byType(LabTimeline));
      expect(timeline.left, greaterThan(0));
      expect(timeline.right, lessThanOrEqualTo(size.width));
      final play = tester.getRect(card('Play'));
      expect(down ? play.bottom : play.right, lessThanOrEqualTo(0));
      expect(card('Play').hitTestable(), findsNothing);
      expect(
        TickerMode.valuesOf(tester.element(find.byType(LabPlayBody))).enabled,
        isFalse,
      );
      expect(
        TickerMode.valuesOf(tester.element(find.byType(LabTimeline))).enabled,
        isTrue,
      );
    });

    testWidgets('a phone $name has a handle on each page, on the edge that '
        'faces the other page', (tester) async {
      await pump(tester, size);
      final timeline = tester.getRect(handleOn('Timeline'));
      final cards = tester.getRect(handleOn('Cards'));
      if (down) {
        // Timeline page: top edge. Cards page: bottom edge, off screen above.
        expect(timeline.top, closeTo(8, 0.5));
        expect(timeline.height, greaterThanOrEqualTo(24));
        expect(cards.bottom, closeTo(0 - 10, 0.5));
      } else {
        expect(timeline.left, closeTo(14, 0.5));
        expect(timeline.width, greaterThanOrEqualTo(24));
        expect(cards.right, closeTo(0 - 14, 0.5));
      }
    });

    testWidgets('a phone $name: each handle is a grip, turned to run across '
        'on the top and bottom, named by the page it shows', (tester) async {
      await pump(tester, size);
      Finder grip(String page) => find.descendant(
        of: handleOn(page),
        matching: find.widgetWithIcon(IconButton, Symbols.drag_indicator),
      );
      expect(grip('Timeline'), findsOneWidget);
      expect(grip('Cards'), findsOneWidget);
      for (final page in ['Timeline', 'Cards']) {
        final turned = find.descendant(
          of: grip(page),
          matching: find.byType(RotatedBox),
        );
        expect(turned, down ? findsOneWidget : findsNothing);
      }
      expect(
        find.descendant(
          of: handleOn('Timeline'),
          matching: find.byWidgetPredicate(
            (w) => w is Tooltip && w.message == 'Cards',
          ),
        ),
        findsOneWidget,
      );
      // A tap on the grip shows the cards.
      await tester.tap(grip('Timeline'));
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isTrue);
    });

    testWidgets('a phone $name: a tap on the handle flips the page and back', (
      tester,
    ) async {
      await pump(tester, size);
      await tester.tap(handleOn('Timeline'));
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isTrue);
      // The Play card fills the page beside its handle.
      final play = tester.getRect(card('Play'));
      if (down) {
        expect(play.width, closeTo(size.width - 16, 0.5));
        expect(play.height, closeTo(size.height - 8 - 10 - 24, 0.5));
      } else {
        expect(play.height, closeTo(size.height - 10 - 12, 0.5));
        expect(play.width, closeTo(size.width - 28 - 24, 0.5));
      }
      expect(find.byType(LabTimeline).hitTestable(), findsNothing);
      expect(card('Play').hitTestable(), findsOneWidget);
      expect(
        TickerMode.valuesOf(tester.element(find.byType(LabTimeline))).enabled,
        isFalse,
      );

      await tester.tap(handleOn('Cards'));
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isFalse);
      expect(find.byType(LabTimeline).hitTestable(), findsOneWidget);
    });

    testWidgets('a phone $name: a tap on a handle mid-slide goes to the page '
        'it names', (tester) async {
      await pump(tester, size);
      await tester.tap(handleOn('Timeline'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      // On its way to the cards page, not yet half way.
      expect(handleOn('Cards').hitTestable(), findsOneWidget);
      expect(onCardsPage(tester), isFalse);
      // The handle named Timeline, on the cards page.
      await tester.tap(handleOn('Cards'));
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isFalse);
      expect(find.byType(LabTimeline).hitTestable(), findsOneWidget);
    });

    testWidgets('a phone $name: a drag past half flips, a short one goes '
        'back, a fast short fling flips', (tester) async {
      await pump(tester, size);
      final extent = down ? size.height : size.width;
      Offset along(double d) => down ? Offset(0, d) : Offset(d, 0);

      await dragHandle(tester, handleOn('Timeline'), along(extent * 0.3));
      expect(onCardsPage(tester), isFalse);

      // Mid-drag both pages are in view, moved with the pointer.
      final gesture = await tester.startGesture(
        tester.getCenter(handleOn('Timeline')),
      );
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(along(extent * 0.05));
        await tester.pump(const Duration(milliseconds: 20));
      }
      final mid = tester.getRect(card('Play'));
      expect(down ? mid.bottom : mid.right, greaterThan(0));
      expect(
        down
            ? tester.getRect(find.byType(LabTimeline)).top
            : tester.getRect(find.byType(LabTimeline)).left,
        lessThan(extent),
      );
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(along(extent * 0.03));
        await tester.pump(const Duration(milliseconds: 20));
      }
      await tester.pump(const Duration(milliseconds: 300));
      await gesture.up();
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isTrue);

      await dragHandle(tester, handleOn('Cards'), along(-extent * 0.3));
      expect(onCardsPage(tester), isTrue);

      await tester.fling(handleOn('Cards'), along(-extent * 0.15), 2000);
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isFalse);

      await tester.fling(handleOn('Timeline'), along(extent * 0.15), 2000);
      await tester.pumpAndSettle();
      expect(onCardsPage(tester), isTrue);
    });
  }

  testWidgets('a desktop window keeps the cards above the timeline, with no '
      'handles and no floating picture', (tester) async {
    await pump(tester, const Size(1600, 900));
    expect(find.byType(LabPip), findsNothing);
    expect(find.byType(LabPageHandle), findsNothing);
    expect(card('Scene'), findsOneWidget);
    expect(find.byType(LabSplitter), findsNWidgets(2));
  });

  group('the floating picture', () {
    testWidgets('starts in the bottom right and settles in the corner it is '
        'dragged to', (tester) async {
      await pump(tester, _phoneUp);
      final start = tester.getRect(pip());
      expect(start.right, closeTo(390 - 20, 0.5));
      expect(start.bottom, closeTo(844 - 22, 0.5));
      expect(start.width, closeTo(390 * 0.4, 0.5));
      expect(start.height, closeTo(start.width * 9 / 16, 0.5));

      final gesture = await tester.startGesture(start.center);
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(-30, -70));
        await tester.pump(const Duration(milliseconds: 16));
      }
      // Held where it was dragged, not yet in the corner.
      expect(tester.getRect(pip()).left, lessThan(start.left - 100));
      await gesture.up();
      await tester.pumpAndSettle();
      final top = tester.getRect(pip());
      expect(top.left, closeTo(20, 0.5));
      expect(top.top, closeTo(20, 0.5));
      // The pages stay as they were.
      expect(onCardsPage(tester), isFalse);
    });

    testWidgets('held still for a long press it peeks until let go, and a '
        'press that moves does not', (tester) async {
      final lab = await pump(tester, _phoneUp);
      final center = tester.getCenter(pip());

      var gesture = await tester.startGesture(center);
      await tester.pump(kLongPressTimeout - const Duration(milliseconds: 50));
      expect(lab.peeks, isEmpty);
      await tester.pump(const Duration(milliseconds: 100));
      expect(lab.peeks, [true]);
      await gesture.up();
      await tester.pump();
      expect(lab.peeks, [true, false]);

      lab.peeks.clear();
      gesture = await tester.startGesture(center);
      await gesture.moveBy(const Offset(-60, 0));
      await tester.pump(kLongPressTimeout + const Duration(milliseconds: 100));
      expect(lab.peeks, isEmpty);
      await gesture.up();
      await tester.pumpAndSettle();
      expect(lab.peeks, isEmpty);
    });

    testWidgets('a mouse shows a grab hand, and a grabbing one while it '
        'drags', (tester) async {
      await pump(tester, _phoneUp);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(tester.getCenter(pip()));
      await tester.pump();
      expect(
        RendererBinding.instance.mouseTracker.debugDeviceActiveCursor(1),
        SystemMouseCursors.grab,
      );
      await mouse.down(tester.getCenter(pip()));
      // The first move past the slop only starts the drag; the picture then
      // follows from there.
      await mouse.moveBy(const Offset(-30, -30));
      await mouse.moveBy(const Offset(-30, -30));
      await tester.pump();
      expect(
        RendererBinding.instance.mouseTracker.debugDeviceActiveCursor(1),
        SystemMouseCursors.grabbing,
      );
      await mouse.up();
      await tester.pumpAndSettle();
    });
  });
}
