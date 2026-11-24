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

VisualizerPresetEntry _entry(String name, {bool builtin = true}) =>
    VisualizerPresetEntry(
      name: name,
      path: builtin ? 'builtin:$name' : '/scenes/$name',
      group: '',
      builtin: builtin,
      inputs: VisualizerSceneInputs.spectrum,
    );

void main() {
  testWidgets('the scene list appears when Rust pushes it, without polling', (
    tester,
  ) async {
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

    // Nothing is asked for, however long it takes Rust to be ready.
    await tester.pump(const Duration(minutes: 1));
    expect(provider.loaded, isFalse);
    expect(provider.scenes, isEmpty);

    pushes.add(
      VisualizerScenesChanged(presets: [_entry('Aurora'), _entry('Bloom')]),
    );
    await tester.pump();

    expect(provider.loaded, isTrue);
    expect(provider.scenes.map((scene) => scene.name), ['Aurora', 'Bloom']);
    expect(provider.scenes.every((scene) => scene.enabled), isTrue);
    expect(store.order, ['builtin:Aurora', 'builtin:Bloom']);
    expect(sent, hasLength(1));
  });

  testWidgets('a pushed change keeps the order and checks the user saved', (
    tester,
  ) async {
    final pushes = StreamController<VisualizerScenesChanged>();
    final store = _MemoryStore();
    final provider = VisualizerScenesProvider.listening(
      scenesChanged: pushes.stream,
      store: store,
      sendScenes: (_) {},
    )..setInputs(VisualizerInputs.full);
    addTearDown(() async {
      provider.dispose();
      await pushes.close();
    });

    pushes.add(
      VisualizerScenesChanged(presets: [_entry('Aurora'), _entry('Bloom')]),
    );
    await tester.pump();
    provider.setEnabled('builtin:Aurora', false);
    provider.reorder(0, 1);

    // A scene is imported: Rust pushes the list again.
    pushes.add(
      VisualizerScenesChanged(
        presets: [
          _entry('Aurora'),
          _entry('Bloom'),
          _entry('Mine', builtin: false),
        ],
      ),
    );
    await tester.pump();

    expect(provider.scenes.map((scene) => scene.name), [
      'Bloom',
      'Aurora',
      'Mine',
    ]);
    expect(provider.scenes.map((scene) => scene.enabled), [true, false, true]);
  });
}
