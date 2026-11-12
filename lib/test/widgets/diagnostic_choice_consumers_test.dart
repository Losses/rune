import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_scenes.dart';
import 'package:rune/screens/settings_laboratory/widgets/settings/fs_self_test_settings.dart';
import 'package:rune/widgets/visualizer/visualizer_surface.dart';

void main() {
  testWidgets('self-test outcomes keep their pass, fail and skip indicators', (
    tester,
  ) async {
    await tester.pumpWidget(
      FluentApp(
        home: ScaffoldPage(
          content: Row(
            children: [
              fsSelfTestOutcomeIcon(FsSelfTestLayerOutcome.passed),
              fsSelfTestOutcomeIcon(FsSelfTestLayerOutcome.failed),
              fsSelfTestOutcomeIcon(FsSelfTestLayerOutcome.skipped),
            ],
          ),
        ),
      ),
    );

    expect(find.byIcon(Symbols.check_circle), findsOneWidget);
    expect(find.byIcon(Symbols.error), findsOneWidget);
    expect(find.byIcon(Symbols.block), findsOneWidget);
  });

  test('visualizer support cases keep scenes gated by GPU support', () {
    final scene = const VisualizerScene(
      path: 'builtin:scene',
      name: 'Scene',
      group: '',
      builtin: true,
      enabled: true,
      inputs: VisualizerSceneInputs.spectrum,
    );
    final scenes = VisualizerScenesProvider.forTest(scenes: [scene]);
    addTearDown(scenes.dispose);

    scenes.applySupport(
      const VisualizerSupportResponse(
        support: VisualizerSupportSupported(adapter: 'GPU'),
        inputs: VisualizerInputs.spectrumOnly,
      ),
    );
    expect(scenes.showVisualizer, isTrue);

    scenes.applySupport(
      const VisualizerSupportResponse(
        support: VisualizerSupportUnsupported(problem: 'No GPU'),
        inputs: VisualizerInputs.spectrumOnly,
      ),
    );
    expect(scenes.showVisualizer, isFalse);
  });

  test('visualizer status keeps the scene path only while running', () {
    VisualizerStatusResponse status(VisualizerRuntimeStatus runtime) =>
        VisualizerStatusResponse(
          runtime: runtime,
          width: 640,
          height: 480,
          targetFps: 60,
          measuredFps: 60,
          presetName: 'Scene',
          presetPath: 'builtin:scene',
          presetCount: 1,
          inputs: VisualizerInputs.full,
        );

    expect(
      visualizerScenePath(
        status(
          const VisualizerRuntimeStatusRunning(
            adapter: 'GPU',
            transport: VisualizerTransport.readback,
          ),
        ),
      ),
      'builtin:scene',
    );
    expect(
      visualizerScenePath(
        status(
          const VisualizerRuntimeStatusStopped(
            adapter: 'GPU',
            transport: VisualizerTransport.readback,
          ),
        ),
      ),
      isEmpty,
    );
    expect(
      visualizerScenePath(
        status(const VisualizerRuntimeStatusNotRunningYet()),
      ),
      isEmpty,
    );
  });
}
