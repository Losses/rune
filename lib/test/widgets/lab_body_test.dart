import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_lab.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/widgets/visualizer/lab/lab_data.dart';
import 'package:rune/widgets/visualizer/lab/lab_overlay.dart';
import 'package:rune/widgets/visualizer/lab/lab_pip_layout.dart';
import 'package:rune/widgets/visualizer/lab/lab_splitter.dart';
import 'package:rune/widgets/visualizer/lab/lab_timeline.dart';
import 'package:rune/widgets/visualizer/lab/lab_views.dart';
import 'package:rune/widgets/visualizer/visualizer_controller.dart';

/// Stands in for the provider with what the lab's body reads and sets.
class _FakeLab extends ChangeNotifier implements VisualizerLabProvider {
  @override
  final now = ValueNotifier<VisualizerLabNow?>(null);

  @override
  final timeline = ValueNotifier<LabTimelineData>(LabTimelineData(length: 180));

  @override
  final rowsModel = LabRowsModel();

  @override
  bool get peeking => false;

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

  final sceneWidths = <(double?, bool)>[];

  @override
  void setSceneWidth(double? width, {bool save = true}) {
    sceneWidths.add((width, save));
    sceneWidth = width;
    notifyListeners();
  }

  final shown = <bool>[];

  @override
  void setBodyShown(bool value) => shown.add(value);

  @override
  void setPeeking(bool peeking) {}

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

  testWidgets('the body tells the provider while it is on screen, and an '
      'open lab with no body on screen is polled at the slow rate', (
    tester,
  ) async {
    final lab = await pump(tester, const Size(1600, 900));
    await tester.pump();
    expect(lab.shown, [true]);
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
    expect(lab.shown, [true, false]);

    expect(
      labPollInterval(open: true, onScreen: true),
      const Duration(milliseconds: 100),
    );
    for (final (open, onScreen) in [
      (true, false),
      (false, true),
      (false, false),
    ]) {
      expect(
        labPollInterval(open: open, onScreen: onScreen),
        const Duration(milliseconds: 500),
        reason: 'open $open, on screen $onScreen',
      );
    }
  });

  testWidgets('the cards fill the row: the Play card takes what the scene '
      'leaves', (tester) async {
    await pump(tester, const Size(1600, 900));
    expect(tester.takeException(), isNull);
    final scene = tester.getRect(card('Scene'));
    final play = tester.getRect(card('Play'));
    final splitter = tester.getRect(
      find.byWidgetPredicate(
        (w) => w is LabSplitter && w.axis == Axis.horizontal,
      ),
    );
    expect(splitter.left, closeTo(scene.right, 0.5));
    expect(play.left, closeTo(splitter.right, 0.5));
    // Right up to the overlay's margin.
    expect(play.right, closeTo(1600 - 14, 0.5));
  });

  testWidgets('dragging between the cards sets the scene\'s width, held '
      'to the room the row has', (tester) async {
    final lab = await pump(tester, const Size(1600, 900));
    final splitter = find.byWidgetPredicate(
      (w) => w is LabSplitter && w.axis == Axis.horizontal,
    );
    final before = tester.getSize(card('Scene')).width;
    final pointer = TestPointer(1, PointerDeviceKind.mouse);
    final at = tester.getCenter(splitter);
    await tester.sendEventToBinding(pointer.down(at));
    await tester.sendEventToBinding(pointer.move(at + const Offset(100, 0)));
    await tester.sendEventToBinding(pointer.up());
    await tester.pump();
    expect(lab.sceneWidths.last, (before + 100, true));
    expect(tester.getSize(card('Scene')).width, closeTo(before + 100, 0.5));

    // A width wider than the row leaves the Play card its least.
    lab.setSceneWidth(5000);
    await tester.pump();
    final row = 1600 - 2 * 14;
    expect(tester.getSize(card('Scene')).width, closeTo(row - 10 - 240, 0.5));
    expect(tester.takeException(), isNull);
  });

  testWidgets('the row does not scroll sideways: in a small window the Play '
      'card ends at the row\'s end', (tester) async {
    await pump(tester, const Size(700, 600));
    expect(tester.takeException(), isNull);
    expect(
      find.byWidgetPredicate(
        (w) => w is Scrollable && w.axisDirection == AxisDirection.right,
      ),
      findsNothing,
    );
    // The narrow window's margin is 8.
    expect(tester.getRect(card('Play')).right, closeTo(700 - 8, 0.5));
  });

  testWidgets('short cards beside a wide scene: the Play card\'s lines '
      'scroll in it rather than spill', (tester) async {
    final lab = await pump(tester, const Size(1600, 900));
    lab.cardsHeight = 96;
    lab.setSceneWidth(5000);
    await tester.pump();
    expect(tester.takeException(), isNull);
    final play = tester.getRect(card('Play'));
    expect(play.width, closeTo(240, 0.5));
    expect(play.height, closeTo(96, 0.5));
    final lines = tester.state<ScrollableState>(
      find
          .descendant(of: card('Play'), matching: find.byType(Scrollable))
          .first,
    );
    expect(lines.position.maxScrollExtent, greaterThan(0));
  });

  testWidgets('each splitter is the whole space between its two parts', (
    tester,
  ) async {
    await pump(tester, const Size(1600, 900));
    final across = tester.getRect(
      find.byWidgetPredicate(
        (w) => w is LabSplitter && w.axis == Axis.horizontal,
      ),
    );
    expect(across.width, labSplitterThickness);
    expect(across.left, closeTo(tester.getRect(card('Scene')).right, 0.5));
    expect(across.right, closeTo(tester.getRect(card('Play')).left, 0.5));
    final down = tester.getRect(
      find.byWidgetPredicate(
        (w) => w is LabSplitter && w.axis == Axis.vertical,
      ),
    );
    expect(down.height, labSplitterThickness);
    expect(down.top, closeTo(tester.getRect(card('Play')).bottom, 0.5));
    expect(
      down.bottom,
      closeTo(tester.getRect(find.byType(LabTimeline)).top, 0.5),
    );
  });
}
