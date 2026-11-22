import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:rune/bindings/bindings.dart';
import 'package:rune/providers/visualizer_scenes.dart';

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

VisualizerPresetEntry _entry(
  String name, {
  String? path,
  VisualizerSceneInputs inputs = VisualizerSceneInputs.spectrum,
}) => VisualizerPresetEntry(
  name: name,
  path: path ?? 'builtin:$name',
  group: '',
  builtin: true,
  inputs: inputs,
);

void main() {
  const wall = kCoverWallScenePath;

  testWidgets(
    'a filtered list moves the chosen scene in both directions',
    (tester) async {
      final pushes = StreamController<VisualizerScenesChanged>();
      final store = _MemoryStore();
      final sent = <SetVisualizerScenesRequest>[];
      final provider = VisualizerScenesProvider.listening(
        scenesChanged: pushes.stream,
        store: store,
        sendScenes: sent.add,
      )..setInputs(VisualizerInputs.spectrumOnly);
      addTearDown(() async {
        provider.dispose();
        await pushes.close();
      });

      pushes.add(
        VisualizerScenesChanged(
          presets: [_entry('Cover Wall', path: wall), _entry('A'), _entry('B')],
        ),
      );
      await tester.pump();

      expect(provider.scenes.map((scene) => scene.name), ['A', 'B']);

      // The hidden cover wall keeps its check while the visible scenes move.
      provider.setEnabled(wall, false);
      expect(store.disabled, [wall]);

      provider.reorder(0, 1);
      await tester.pump();

      expect(provider.scenes.map((scene) => scene.name), ['B', 'A']);
      expect(store.order, [wall, 'builtin:B', 'builtin:A']);
      expect(store.disabled, [wall]);
      expect(sent.last.order, store.order);
      expect(sent.last.disabled, store.disabled);

      provider.reorder(1, 0);
      await tester.pump();

      expect(provider.scenes.map((scene) => scene.name), ['A', 'B']);
      expect(store.order, [wall, 'builtin:A', 'builtin:B']);
      expect(sent.last.order, store.order);

      // The cover wall returns in its place, still unchecked.
      provider.setInputs(VisualizerInputs.full);
      expect(provider.scenes.map((scene) => scene.path), [
        wall,
        'builtin:A',
        'builtin:B',
      ]);
      expect(provider.scenes.first.enabled, isFalse);
    },
  );

  testWidgets('a filtered list keeps the scenes it hides in their places', (
    tester,
  ) async {
    final pushes = StreamController<VisualizerScenesChanged>();
    final store = _MemoryStore();
    final sent = <SetVisualizerScenesRequest>[];
    final provider = VisualizerScenesProvider.listening(
      scenesChanged: pushes.stream,
      store: store,
      sendScenes: sent.add,
    )..setInputs(VisualizerInputs.spectrumOnly);
    addTearDown(() async {
      provider.dispose();
      await pushes.close();
    });

    pushes.add(
      VisualizerScenesChanged(
        presets: [
          _entry('Cover Wall', path: wall),
          _entry('A'),
          _entry('Chart', inputs: VisualizerSceneInputs.spectrumAndChart),
          _entry('B'),
        ],
      ),
    );
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.name), ['A', 'B']);

    provider.setEnabled('builtin:Chart', false);
    provider.reorder(0, 1);
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.name), ['B', 'A']);
    expect(store.order, [wall, 'builtin:B', 'builtin:Chart', 'builtin:A']);
    expect(store.disabled, ['builtin:Chart']);
    expect(sent.last.order, store.order);
    expect(sent.last.disabled, store.disabled);

    provider.setInputs(VisualizerInputs.full);
    expect(provider.scenes.map((scene) => scene.name), [
      'Cover Wall',
      'B',
      'Chart',
      'A',
    ]);
    expect(provider.scenes[2].enabled, isFalse);
  });

  testWidgets('a full list moves its first and last scenes', (tester) async {
    final pushes = StreamController<VisualizerScenesChanged>();
    final store = _MemoryStore();
    final sent = <SetVisualizerScenesRequest>[];
    final provider = VisualizerScenesProvider.listening(
      scenesChanged: pushes.stream,
      store: store,
      sendScenes: sent.add,
    )..setInputs(VisualizerInputs.full);
    addTearDown(() async {
      provider.dispose();
      await pushes.close();
    });

    pushes.add(
      VisualizerScenesChanged(
        presets: [_entry('Cover Wall', path: wall), _entry('A'), _entry('B')],
      ),
    );
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.path), [
      wall,
      'builtin:A',
      'builtin:B',
    ]);

    provider.reorder(0, 2);
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.path), [
      'builtin:A',
      'builtin:B',
      wall,
    ]);
    expect(store.order, ['builtin:A', 'builtin:B', wall]);
    expect(sent.last.order, store.order);

    provider.reorder(2, 0);
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.path), [
      wall,
      'builtin:A',
      'builtin:B',
    ]);
    expect(store.order, [wall, 'builtin:A', 'builtin:B']);
    expect(sent.last.order, store.order);
  });
}
