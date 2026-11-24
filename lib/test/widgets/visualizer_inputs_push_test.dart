import 'dart:async';
import 'dart:typed_data';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:provider/provider.dart';
import 'package:reorderables/reorderables.dart';
import 'package:rinf/rinf.dart';

import '../../bindings/bindings.dart';
import '../../l10n/app_localizations.dart';
import '../../providers/visualizer_scenes.dart';
import '../../screens/settings_theme/widgets/visualizer_scenes_setting.dart';
import '../../widgets/playback_controller/cover_wall_button.dart';

class _MemoryStore implements VisualizerSceneStore {
  List<String>? order;
  List<String>? disabled;

  @override
  Future<List<String>?> loadOrder() async => order;

  @override
  Future<List<String>?> loadDisabled() async => disabled;

  @override
  Future<void> save(List<String> order, List<String> disabled) async {
    this.order = order;
    this.disabled = disabled;
  }
}

const _scenes = VisualizerScenesChanged(
  presets: [
    VisualizerPresetEntry(
      name: 'Chart',
      path: 'builtin:Chart',
      group: '',
      builtin: true,
      inputs: VisualizerSceneInputs.spectrumAndChart,
    ),
    VisualizerPresetEntry(
      name: 'Spectrum',
      path: 'builtin:Spectrum',
      group: '',
      builtin: true,
      inputs: VisualizerSceneInputs.spectrum,
    ),
    VisualizerPresetEntry(
      name: 'Cover Wall',
      path: kCoverWallScenePath,
      group: '',
      builtin: false,
      inputs: VisualizerSceneInputs.spectrum,
    ),
  ],
);

const _gpu = VisualizerSupportSupported(adapter: 'test');

/// A provider that hears the scenes and the source's inputs as Rust pushes
/// them.
Future<(VisualizerScenesProvider, StreamController<VisualizerInputsChanged>)>
_provider(WidgetTester tester) async {
  final scenes = StreamController<VisualizerScenesChanged>();
  final inputs = StreamController<VisualizerInputsChanged>();
  final provider = VisualizerScenesProvider.listening(
    scenesChanged: scenes.stream,
    store: _MemoryStore(),
    sendScenes: (_) {},
    inputsChanged: inputs.stream,
  );
  addTearDown(() {
    provider.dispose();
    // Neither is awaited: a stream nobody listens to any more never says it
    // is done.
    unawaited(scenes.close());
    unawaited(inputs.close());
  });
  scenes.add(_scenes);
  await tester.pump();
  return (provider, inputs);
}

void main() {
  testWidgets('the visualizer follows the pushed inputs with none running', (
    tester,
  ) async {
    final (provider, inputs) = await _provider(tester);
    // The machine has a GPU, and no source has been pushed yet.
    provider.applySupport(
      const VisualizerSupportResponse(
        support: _gpu,
        inputs: VisualizerInputs.none,
      ),
    );
    await tester.pumpWidget(
      ChangeNotifierProvider<VisualizerScenesProvider>.value(
        value: provider,
        child: const FluentApp(home: CoverWallButton(shadows: null)),
      ),
    );
    Future<void> push(VisualizerInputs given) async {
      inputs.add(VisualizerInputsChanged(inputs: given));
      await tester.pumpAndSettle();
    }

    await push(VisualizerInputs.full);
    expect(find.byIcon(Symbols.graphic_eq), findsOneWidget);

    // The jukebox's feed drops: the cover wall stands in.
    await push(VisualizerInputs.none);
    expect(find.byIcon(Symbols.photo_frame), findsOneWidget);

    // The feed returns, and the visualizer comes back.
    await push(VisualizerInputs.full);
    expect(find.byIcon(Symbols.graphic_eq), findsOneWidget);
  });

  testWidgets('the settings arrange every scene while the source gives none', (
    tester,
  ) async {
    final (provider, inputs) = await _provider(tester);
    provider.applySupport(
      const VisualizerSupportResponse(
        support: _gpu,
        inputs: VisualizerInputs.none,
      ),
    );
    // A platform host plays the track, and its audio never reaches the player.
    inputs.add(const VisualizerInputsChanged(inputs: VisualizerInputs.none));
    await tester.pumpWidget(
      ChangeNotifierProvider<VisualizerScenesProvider>.value(
        value: provider,
        child: const FluentApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: Locale('en'),
          home: ScaffoldPage(
            content: SingleChildScrollView(child: VisualizerScenesSetting()),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Scenes'));
    await tester.pumpAndSettle();
    List<String> rows() => [
      for (final checkbox in tester.widgetList<Checkbox>(find.byType(Checkbox)))
        ((checkbox.content as Text).textSpan! as TextSpan).children!
            .cast<TextSpan>()
            .single
            .text!,
    ];

    expect(rows(), ['Cover Wall', 'Chart', 'Spectrum']);

    // A drag moves the scene in the row dragged.
    tester
        .widget<ReorderableColumn>(find.byType(ReorderableColumn))
        .onReorder(1, 2);
    await tester.pump();
    expect(rows(), ['Cover Wall', 'Spectrum', 'Chart']);

    // A file plays again, and every scene can run in the new order.
    inputs.add(const VisualizerInputsChanged(inputs: VisualizerInputs.full));
    await tester.pumpAndSettle();
    expect(rows(), ['Cover Wall', 'Spectrum', 'Chart']);
  });

  testWidgets('a pushed source outweighs what the visualizer reported', (
    tester,
  ) async {
    final (provider, inputs) = await _provider(tester);
    // Before any push, what the visualizer reports when asked counts.
    provider.setInputs(VisualizerInputs.full);
    expect(provider.inputs, VisualizerInputs.full);

    inputs.add(const VisualizerInputsChanged(inputs: VisualizerInputs.none));
    await tester.pumpAndSettle();
    expect(provider.inputs, VisualizerInputs.none);

    // A status or support report asked for before the change arrives late:
    // the push decides.
    provider.setInputs(VisualizerInputs.full);
    provider.applySupport(
      const VisualizerSupportResponse(
        support: _gpu,
        inputs: VisualizerInputs.full,
      ),
    );
    expect(provider.inputs, VisualizerInputs.none);
  });

  test('a push sent before the provider existed still counts', () {
    VisualizerInputsChanged.latestRustSignal = RustSignalPack(
      const VisualizerInputsChanged(inputs: VisualizerInputs.none),
      Uint8List(0),
    );
    addTearDown(() => VisualizerInputsChanged.latestRustSignal = null);
    final provider = VisualizerScenesProvider();
    addTearDown(provider.dispose);
    provider.setInputs(VisualizerInputs.full);

    expect(provider.inputs, VisualizerInputs.none);
  });
}
