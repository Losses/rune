import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/widgets/playback_controller/cover_wall_button.dart';

void main() {
  testWidgets('spectrum-only visualizer hides when its only scene is disabled', (
    tester,
  ) async {
    final scenes = VisualizerScenesProvider.forTest(
      scenes: [
        const VisualizerScene(
          path: 'spectrum-scene',
          name: 'Spectrum scene',
          group: '',
          builtin: true,
          enabled: false,
          inputs: VisualizerSceneInputs.spectrum,
        ),
      ],
    );
    addTearDown(scenes.dispose);
    scenes.setInputs(VisualizerInputs.spectrumOnly);

    await tester.pumpWidget(
      ChangeNotifierProvider<VisualizerScenesProvider>.value(
        value: scenes,
        child: FluentApp(
          home: ScaffoldPage(
            content: const CoverWallButton(shadows: null),
          ),
        ),
      ),
    );

    expect(find.byIcon(Symbols.graphic_eq), findsNothing);
    expect(find.byIcon(Symbols.photo_frame), findsOneWidget);
  });

  testWidgets('visualizer entry and scene controls follow source inputs', (
    tester,
  ) async {
    final scenes = VisualizerScenesProvider.forTest(
      scenes: [
        const VisualizerScene(
          path: 'spectrum-scene',
          name: 'Spectrum scene',
          group: '',
          builtin: true,
          enabled: true,
          inputs: VisualizerSceneInputs.spectrum,
        ),
        const VisualizerScene(
          path: 'chart-scene',
          name: 'Chart scene',
          group: '',
          builtin: true,
          enabled: true,
          inputs: VisualizerSceneInputs.spectrumAndChart,
        ),
        const VisualizerScene(
          path: kCoverWallScenePath,
          name: 'Cover Wall',
          group: '',
          builtin: false,
          enabled: true,
          inputs: VisualizerSceneInputs.spectrum,
        ),
      ],
    );
    addTearDown(scenes.dispose);

    Future<void> showState(VisualizerInputs inputs) async {
      scenes.setInputs(inputs);
      await tester.pumpWidget(
        ChangeNotifierProvider<VisualizerScenesProvider>.value(
          value: scenes,
          child: FluentApp(
            home: Builder(
              builder: (context) => ScaffoldPage(
                content: Column(
                  children: [
                    const CoverWallButton(shadows: null),
                    for (final scene in context
                        .watch<VisualizerScenesProvider>()
                        .scenes)
                      Text(scene.name),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
    }

    await showState(VisualizerInputs.none);
    expect(find.byIcon(Symbols.photo_frame), findsOneWidget);
    expect(find.text('Spectrum scene'), findsNothing);
    expect(find.text('Chart scene'), findsNothing);

    await showState(VisualizerInputs.spectrumOnly);
    expect(find.byIcon(Symbols.graphic_eq), findsOneWidget);
    expect(find.text('Spectrum scene'), findsOneWidget);
    expect(find.text('Chart scene'), findsNothing);
    expect(find.text('Cover Wall'), findsNothing);

    await showState(VisualizerInputs.full);
    expect(find.byIcon(Symbols.graphic_eq), findsOneWidget);
    expect(find.text('Spectrum scene'), findsOneWidget);
    expect(find.text('Chart scene'), findsOneWidget);
  });
}
