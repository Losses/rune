import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter/rendering.dart' show RenderParagraph;
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';

import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_lab.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/widgets/visualizer/lab/lab_cards.dart';
import 'package:rune/widgets/visualizer/lab/lab_equal_height_row.dart';
import 'package:rune/widgets/visualizer/lab/lab_menu.dart';
import 'package:rune/widgets/visualizer/lab/lab_overlay.dart';
import 'package:rune/widgets/visualizer/lab/lab_segmented.dart';
import 'package:rune/widgets/visualizer/visualizer_controller.dart';

/// Stands in for the provider: the Play card reads `now` and sends commands
/// through `play`.
class _FakeLab extends ChangeNotifier implements VisualizerLabProvider {
  @override
  final now = ValueNotifier<VisualizerLabNow?>(null);

  final played = <VisualizerLabCommand>[];

  @override
  void play(VisualizerLabCommand command, {String? markLabel}) =>
      played.add(command);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

VisualizerLabNow _now({
  bool director = true,
  bool looks = true,
  VisualizerLabLevel? forcedLevel,
  String closeName = 'Close',
  bool shotHeld = false,
}) => VisualizerLabNow(
  director: director
      ? VisualizerLabDirector(
          shotIndex: 1,
          held: shotHeld,
          shots: [
            const VisualizerLabShot(name: 'Wide', size: 'wide'),
            VisualizerLabShot(name: closeName, size: 'close'),
          ],
        )
      : null,
  looks: looks
      ? const VisualizerLabLooks(names: ['Warm', 'Cold'], on: 0, held: false)
      : null,
  effects: const VisualizerLabEffects(
    cut: ['flash', 'glitch'],
    lens: ['flare', 'leak'],
  ),
  forcedLevel: forcedLevel,
);

void main() {
  group('the Scene picture', () {
    final controller = VisualizerController();

    Widget picture(Widget Function(Widget) frame) => FluentApp(
      home: Align(
        alignment: Alignment.topLeft,
        child: frame(
          LabScenePreview(
            controller: controller,
            peeking: false,
            onHold: (_) {},
          ),
        ),
      ),
    );

    setUp(() => controller.textureAspect.value = 16 / 9);

    testWidgets('asks for 160 when the card is free, as wide as the shape', (
      tester,
    ) async {
      await tester.pumpWidget(picture((child) => child));
      final size = tester.getSize(find.byType(LabScenePreview));
      expect(size.height, labScenePictureHeight);
      expect(size.width, closeTo(labScenePictureHeight * 16 / 9, 1e-6));

      controller.textureAspect.value = 1;
      await tester.pump();
      expect(
        tester.getSize(find.byType(LabScenePreview)),
        const Size(labScenePictureHeight, labScenePictureHeight),
      );
    });

    testWidgets('fills a tight height, taller or shorter than 160', (
      tester,
    ) async {
      for (final h in [90.0, 300.0]) {
        await tester.pumpWidget(
          picture((child) => SizedBox(height: h, child: child)),
        );
        final size = tester.getSize(find.byType(LabScenePreview));
        expect(size.height, h);
        expect(size.width, closeTo(h * 16 / 9, 1e-6));
      }
    });

    testWidgets('takes no more than the room when free', (tester) async {
      await tester.pumpWidget(
        picture(
          (child) => ConstrainedBox(
            constraints: const BoxConstraints(maxHeight: 100),
            child: child,
          ),
        ),
      );
      expect(tester.getSize(find.byType(LabScenePreview)).height, 100);
    });

    testWidgets('given a width, the picture fits whole on black', (
      tester,
    ) async {
      controller.textureId.value = 7;
      addTearDown(() => controller.textureId.value = null);
      await tester.pumpWidget(
        FluentApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              height: 160,
              child: LabScenePreview(
                controller: controller,
                width: 400,
                peeking: false,
                onHold: (_) {},
              ),
            ),
          ),
        ),
      );
      final shown = tester.getRect(find.byType(LabScenePreview));
      expect(shown.size, const Size(400, 160));
      // 16:9 at 160 px high, in the middle, black on both sides.
      final picture = tester.getRect(find.byType(Texture));
      expect(picture.height, 160);
      expect(picture.width, closeTo(160 * 16 / 9, 0.01));
      expect(picture.center.dx, closeTo(shown.center.dx, 0.01));
      // Narrower than its shape, it is black above and below.
      await tester.pumpWidget(
        FluentApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              height: 160,
              child: LabScenePreview(
                controller: controller,
                width: 160,
                peeking: false,
                onHold: (_) {},
              ),
            ),
          ),
        ),
      );
      expect(tester.getSize(find.byType(Texture)).width, 160);
      expect(tester.getSize(find.byType(Texture)).height, closeTo(90, 0.01));
    });

    testWidgets('a texture fills the picture exactly', (tester) async {
      controller.textureId.value = 7;
      addTearDown(() => controller.textureId.value = null);
      await tester.pumpWidget(
        picture((child) => SizedBox(height: 120, child: child)),
      );
      expect(
        tester.getSize(find.byType(Texture)),
        tester.getSize(find.byType(LabScenePreview)),
      );
      controller.textureId.value = null;
    });
  });

  group('the card chrome', () {
    const body = ValueKey('body-box');

    Widget card(double height, {bool framed = true}) => FluentApp(
      home: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          height: height,
          child: LabCard(
            label: 'Scene',
            framed: framed,
            child: const SizedBox(key: body, width: 100, height: 10),
          ),
        ),
      ),
    );

    testWidgets('a card has no title line over its body', (tester) async {
      await tester.pumpWidget(card(labCompactCardHeight));
      expect(find.text('Scene'), findsNothing);
      final box = tester.getRect(find.byType(LabCard));
      final inner = tester.getRect(find.byKey(body));
      // The 1 px border and the margin.
      expect(inner.top - box.top, 17);
      expect(inner.left - box.left, 17);
    });

    testWidgets('a framed card is the app\'s card, in the theme\'s colors', (
      tester,
    ) async {
      await tester.pumpWidget(card(labCompactCardHeight));
      final frame = tester.widget<Card>(
        find.descendant(of: find.byType(LabCard), matching: find.byType(Card)),
      );
      expect(frame.backgroundColor, isNull);
      expect(frame.borderColor, isNull);
    });

    testWidgets('under 150 px the margin is narrower', (tester) async {
      await tester.pumpWidget(card(labCompactCardHeight - 1));
      final box = tester.getRect(find.byType(LabCard));
      final inner = tester.getRect(find.byKey(body));
      expect(inner.top - box.top, 11);
      expect(inner.left - box.left, 11);
    });

    testWidgets('a card without a frame is its body alone', (tester) async {
      await tester.pumpWidget(card(labCompactCardHeight, framed: false));
      final box = tester.getRect(find.byType(LabCard));
      final inner = tester.getRect(find.byKey(body));
      expect(inner.topLeft, box.topLeft);
      expect(inner.width, box.width);
      expect(
        find.descendant(
          of: find.byType(LabCard),
          matching: find.byType(Container),
        ),
        findsNothing,
      );
    });
  });

  group('the card bodies', () {
    // The Play card in a row of one height, as the overlay builds it.
    Widget cards(_FakeLab lab, double height) => FluentApp(
      home: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: LabEqualHeightRow(
          gap: 10,
          maxHeight: 400,
          height: height,
          children: [
            LabCard(
              label: 'Play',
              child: LabPlayBody(lab: lab),
            ),
          ],
        ),
      ),
    );

    final states = <String, VisualizerLabNow?>{
      'nothing drawn': null,
      'everything': _now(),
      'no shots': _now(director: false),
      'no looks': _now(looks: false),
      'no shots or looks': _now(director: false, looks: false),
    };

    for (final MapEntry(key: name, value: now) in states.entries) {
      for (final height in [96.0, 150.0, 300.0]) {
        testWidgets('$name at $height px has no overflow', (tester) async {
          tester.view.physicalSize = const Size(1600, 900);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final lab = _FakeLab()..now.value = now;
          await tester.pumpWidget(cards(lab, height));
          await tester.pump();
          expect(tester.takeException(), isNull);
          expect(tester.getSize(find.byType(LabCard).first).height, height);
        });
      }
    }

    testWidgets('a short card shows more columns than a tall one', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 300));
      final tall = tester.getSize(find.byType(LabCard)).width;
      await tester.pumpWidget(cards(lab, 96));
      final short = tester.getSize(find.byType(LabCard)).width;
      expect(short, greaterThan(tall));
    });

    testWidgets('the buttons still play', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 300));
      await tester.tap(find.text('Flare').first);
      await tester.pump(const Duration(seconds: 1));
      expect(lab.played.single, isA<VisualizerLabCommandFlare>());
    });

    testWidgets('the Camera line cuts plain, or with each transition', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 300));
      // The transitions are in the Camera line, under Cut, not a line of
      // their own.
      expect(find.text('Dressed Cut'), findsNothing);
      final cut = tester.getRect(find.text('Cut'));
      final flash = tester.getRect(find.text('Flash'));
      expect(flash.top, greaterThan(cut.bottom));
      expect(flash.top, lessThan(tester.getRect(find.text('Flare')).top));

      await tester.tap(find.text('Cut'));
      await tester.pump(const Duration(seconds: 1));
      expect(lab.played.single, isA<VisualizerLabCommandCut>());
      // A transition the scene's cuts do not wear is dimmed, and plays.
      double opacityOf(String name) => tester
          .widget<Opacity>(
            find
                .ancestor(of: find.text(name), matching: find.byType(Opacity))
                .first,
          )
          .opacity;
      expect(opacityOf('Flash'), 1);
      expect(opacityOf('Fade'), lessThan(1));
      await tester.tap(find.text('Fade'));
      await tester.pump(const Duration(seconds: 1));
      expect(
        lab.played.last,
        isA<VisualizerLabCommandCutDressed>().having(
          (c) => c.transition,
          'transition',
          VisualizerLabTransition.fade,
        ),
      );
    });

    testWidgets('each part of the Play card is a named line of its own', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 400));
      double middle(Finder f) => tester.getCenter(f.first).dy;
      // A text's baseline, read outside a layout the way intrinsics are.
      double baseline(Finder f) {
        final box = tester.renderObject<RenderBox>(f.first);
        RenderObject.debugCheckingIntrinsics = true;
        try {
          return box.localToGlobal(Offset.zero).dy +
              box.getDistanceToBaseline(TextBaseline.alphabetic)!;
        } finally {
          RenderObject.debugCheckingIntrinsics = false;
        }
      }

      Finder auto(Finder control) =>
          find.descendant(of: control, matching: find.text('Auto'));
      final lists = find.byType(LabChoiceButton<int>);

      // The name stands on the baseline of the first line of buttons or of
      // a choice, and in the middle of a list.
      for (final (name, control) in [
        ('Camera', find.text('Cut')),
        ('Lens', find.text('Flare')),
        ('Level', auto(find.byType(LabSegmented<VisualizerLabLevel?>))),
      ]) {
        expect(
          (baseline(find.text(name)) - baseline(control)).abs(),
          lessThan(0.5),
          reason: name,
        );
      }
      for (final (name, control) in [
        ('Look', auto(lists.at(0))),
        ('Shot', auto(lists.at(1))),
      ]) {
        expect(
          (middle(find.text(name)) - middle(control)).abs(),
          lessThan(1),
          reason: name,
        );
      }
      // The look and the shot no longer share a line.
      expect(
        middle(find.text('Shot')),
        greaterThan(
          tester.getRect(find.byType(LabChoiceButton<int>).at(0)).bottom,
        ),
      );
    });

    testWidgets('the names of its lines are in the app\'s subtitle color', (
      tester,
    ) async {
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 400));
      final theme = FluentTheme.of(tester.element(find.text('Camera')));
      for (final name in ['Camera', 'Lens', 'Look', 'Shot', 'Level']) {
        final style = tester.widget<Text>(find.text(name)).style;
        expect(style?.color, theme.inactiveColor.withAlpha(160), reason: name);
        expect(style?.fontSize, theme.typography.caption?.fontSize);
      }
    });

    testWidgets('a list on Auto says Auto alone, not what is on now', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 400));
      final lists = find.byType(LabChoiceButton<int>);
      for (var i = 0; i < 2; i++) {
        expect(
          find.descendant(of: lists.at(i), matching: find.text('Auto')),
          findsOneWidget,
        );
      }
      // The look on is Warm and the shot on Close: neither is named.
      expect(find.textContaining('Warm'), findsNothing);
      expect(find.textContaining('Close'), findsNothing);
    });

    testWidgets('a list holds what its menu picks, Auto for none', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 400));
      await tester.tap(find.byType(LabChoiceButton<int>).at(1));
      await tester.pumpAndSettle();
      // Auto checked, as nothing is held.
      expect(find.byIcon(Symbols.check), findsOneWidget);
      await tester.tap(find.text('Wide (wide)').last);
      await tester.pumpAndSettle();
      expect(lab.played.last, const VisualizerLabCommandShot(index: 0));
    });

    testWidgets(
      'a long name stays on one line, in a list as wide as its line',
      (tester) async {
        tester.view.physicalSize = const Size(1600, 900);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        final lab = _FakeLab()
          ..now.value = _now(
            closeName: 'through the rings and out the far side',
            shotHeld: true,
          );
        await tester.pumpWidget(cards(lab, 400));
        final label = 'through the rings and out the far side (close)';
        final text = tester.renderObject<RenderParagraph>(find.text(label));
        expect(text.didExceedMaxLines, isTrue);
        // One line of the 14 px test font, not two.
        expect(tester.getSize(find.text(label)).height, lessThan(20));
        // The list takes the rest of the line, as the choice of a level does.
        final shot = tester.getRect(find.byType(LabChoiceButton<int>).at(1));
        final level = tester.getRect(
          find.byType(LabSegmented<VisualizerLabLevel?>),
        );
        expect(shot.left, closeTo(level.left, 0.5));
        expect(shot.right, closeTo(level.right, 0.5));
      },
    );

    testWidgets('the Play card holds the level: Auto, No Beat and No Chart', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(cards(lab, 300));
      final play = find.ancestor(
        of: find.byType(LabPlayBody),
        matching: find.byType(LabCard),
      );
      expect(
        find.descendant(of: play, matching: find.text('Level')),
        findsOneWidget,
      );
      // One choice of the three, as the Display's strips are.
      expect(
        find.descendant(
          of: play,
          matching: find.byType(LabSegmented<VisualizerLabLevel?>),
        ),
        findsOneWidget,
      );
      final level = find.byType(LabSegmented<VisualizerLabLevel?>);
      for (final label in ['Auto', 'No Beat', 'No Chart']) {
        expect(
          find.descendant(of: level, matching: find.text(label)),
          findsOneWidget,
        );
      }
      for (final label in ['A', 'B', 'C']) {
        expect(
          find.descendant(of: play, matching: find.text(label)),
          findsNothing,
        );
      }

      await tester.tap(
        find.descendant(
          of: find.byType(LabPlayBody),
          matching: find.text('No Beat'),
        ),
      );
      await tester.pump(const Duration(seconds: 1));
      expect(
        lab.played.single,
        const VisualizerLabCommandForceLevel(level: VisualizerLabLevel.b),
      );
    });
  });

  group('the scene picker', () {
    const scenes = [
      VisualizerScene(
        path: kCoverWallScenePath,
        name: 'Cover Wall',
        group: '',
        builtin: true,
        enabled: true,
      ),
      VisualizerScene(
        path: 'builtin:torii-sea',
        name: 'Torii Sea',
        group: '',
        builtin: true,
        enabled: true,
      ),
      VisualizerScene(
        path: 'builtin:neon-street',
        name: 'Neon Street',
        group: '',
        builtin: true,
        enabled: false,
      ),
    ];

    Widget picker(List<String> picked) => FluentApp(
      home: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: 400,
          child: LabScenePicker(
            scenes: scenes,
            current: 'builtin:torii-sea',
            name: 'Torii Sea',
            onPicked: picked.add,
          ),
        ),
      ),
    );

    testWidgets('names the scene on screen and shows another from its menu', (
      tester,
    ) async {
      final picked = <String>[];
      await tester.pumpWidget(picker(picked));
      expect(find.text('Torii Sea'), findsOneWidget);

      await tester.tap(find.text('Torii Sea'));
      await tester.pumpAndSettle();
      // The scenes of the visualizer, not the cover wall; the one on screen
      // checked; one left out of the turn dimmed.
      expect(find.text('Cover Wall'), findsNothing);
      expect(find.text('Neon Street'), findsOneWidget);
      expect(find.byIcon(Symbols.check), findsOneWidget);
      final check = tester.getCenter(find.byIcon(Symbols.check));
      expect(
        (check.dy - tester.getCenter(find.text('Torii Sea').last).dy).abs(),
        lessThan(4),
      );
      double opacityOf(String name) => tester
          .widget<Opacity>(
            find
                .ancestor(
                  of: find.text(name).last,
                  matching: find.byType(Opacity),
                )
                .first,
          )
          .opacity;
      expect(opacityOf('Neon Street'), lessThan(1));
      expect(opacityOf('Torii Sea'), 1);

      await tester.tap(find.text('Neon Street'));
      await tester.pumpAndSettle();
      expect(picked, ['builtin:neon-street']);
    });

    testWidgets('its menu is the lab\'s: dark over a light app, on the app\'s '
        'own navigator', (tester) async {
      final page = GlobalKey<NavigatorState>();
      await tester.pumpWidget(
        FluentApp(
          theme: FluentThemeData(brightness: Brightness.light),
          // The page's navigator, under the app's, as the visualizer's is.
          home: Navigator(
            key: page,
            onGenerateRoute: (_) => PageRouteBuilder(
              pageBuilder: (context, _, _) => FluentTheme(
                data: FluentThemeData(brightness: Brightness.dark),
                child: Align(
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    width: 400,
                    child: LabScenePicker(
                      scenes: scenes,
                      current: 'builtin:torii-sea',
                      name: 'Torii Sea',
                      onPicked: (_) {},
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Torii Sea'));
      await tester.pumpAndSettle();
      final menu = find.byType(MenuFlyout);
      expect(menu, findsOneWidget);
      expect(FluentTheme.of(tester.element(menu)).brightness, Brightness.dark);
      expect(
        find.descendant(of: find.byKey(page), matching: menu),
        findsNothing,
      );
    });

    testWidgets('the Play card ends its lists where its widest buttons do', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      // Half-size text, so that the widest row of buttons is on one line.
      await tester.pumpWidget(
        FluentApp(
          home: MediaQuery(
            data: const MediaQueryData(textScaler: TextScaler.linear(0.5)),
            child: Align(
              alignment: Alignment.topLeft,
              child: SizedBox(
                height: 400,
                child: LabPlayBody(
                  lab: lab,
                  header: LabScenePicker(
                    scenes: scenes,
                    current: 'builtin:torii-sea',
                    name: 'Torii Sea',
                    onPicked: (_) {},
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      Rect buttonOf(String label) => tester.getRect(
        find
            .ancestor(of: find.text(label), matching: find.byType(Button))
            .first,
      );
      final widest = buttonOf('Dissolve').right;
      for (final list in [
        find.byType(LabChoiceButton<int>).at(0),
        find.byType(LabChoiceButton<int>).at(1),
        find.byType(LabSegmented<VisualizerLabLevel?>),
      ]) {
        expect(tester.getRect(list).right, closeTo(widest, 1));
      }
      expect(
        tester.getSize(find.byType(LabPlayBody)).width,
        closeTo(widest - tester.getTopLeft(find.byType(LabPlayBody)).dx, 1),
      );
      // The scene's name starts where the names of the lines do.
      expect(
        tester.getTopLeft(find.text('Torii Sea')).dx,
        closeTo(tester.getTopLeft(find.text('Camera')).dx, 0.5),
      );
    });

    testWidgets('opens from anywhere on its button, in a card that makes '
        'room for it', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(
        FluentApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              height: 400,
              child: LabCard(
                label: 'Play',
                hang: labTitleInset,
                child: LabPlayBody(
                  lab: lab,
                  header: LabScenePicker(
                    scenes: scenes,
                    current: 'builtin:torii-sea',
                    name: 'Torii Sea',
                    onPicked: (_) {},
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      // The name stands where the names of the lines do.
      expect(
        tester.getTopLeft(find.text('Torii Sea')).dx,
        closeTo(tester.getTopLeft(find.text('Camera')).dx, 0.5),
      );
      // A press at the button's top left corner opens the menu.
      final button = tester.getRect(
        find.ancestor(
          of: find.text('Torii Sea'),
          matching: find.byType(IconButton),
        ),
      );
      await tester.tapAt(button.topLeft + const Offset(2, 2));
      await tester.pumpAndSettle();
      expect(find.text('Neon Street'), findsOneWidget);
    });

    testWidgets('names the scene as the app titles its settings cards', (
      tester,
    ) async {
      await tester.pumpWidget(picker([]));
      final name = find.text('Torii Sea');
      expect(
        tester.widget<Text>(name).style,
        FluentTheme.of(tester.element(name)).typography.subtitle,
      );
    });

    testWidgets('with no scenes, names the scene and opens nothing', (
      tester,
    ) async {
      await tester.pumpWidget(
        FluentApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              width: 400,
              child: LabScenePicker(
                scenes: const [],
                current: null,
                name: 'Torii Sea',
                onPicked: (_) {},
              ),
            ),
          ),
        ),
      );
      expect(tester.takeException(), isNull);
      expect(find.text('Torii Sea'), findsOneWidget);
      await tester.tap(find.text('Torii Sea'));
      await tester.pumpAndSettle();
      expect(find.byType(MenuFlyout), findsNothing);
    });

    testWidgets('heads the Play card', (tester) async {
      tester.view.physicalSize = const Size(1600, 900);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final lab = _FakeLab()..now.value = _now();
      await tester.pumpWidget(
        FluentApp(
          home: Align(
            alignment: Alignment.topLeft,
            child: SizedBox(
              height: 400,
              child: LabPlayBody(
                lab: lab,
                header: LabScenePicker(
                  scenes: scenes,
                  current: 'builtin:torii-sea',
                  name: 'Torii Sea',
                  onPicked: (_) {},
                ),
              ),
            ),
          ),
        ),
      );
      final title = tester.getRect(find.text('Torii Sea'));
      expect(title.bottom, lessThan(tester.getRect(find.text('Camera')).top));
    });
  });
}
