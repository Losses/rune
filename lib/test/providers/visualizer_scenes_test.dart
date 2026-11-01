import 'package:flutter_test/flutter_test.dart';
import 'package:rune/providers/visualizer_scenes.dart';

void main() {
  const wall = kCoverWallScenePath;

  test('the first run checks every scene, the cover wall first', () {
    final (order, disabled) = reconcileScenes(
      listed: ['builtin:A', wall, 'builtin:B'],
      storedOrder: null,
      storedDisabled: null,
    );
    expect(order, [wall, 'builtin:A', 'builtin:B']);
    expect(disabled, isEmpty);
  });

  test('a scene missing from a listing keeps its place and its check', () {
    final stored = ['/scenes/Mine', 'builtin:A', wall];
    // The folder of imported scenes was not scanned yet.
    final (order, disabled) = reconcileScenes(
      listed: ['builtin:A', wall],
      storedOrder: stored,
      storedDisabled: ['/scenes/Mine'],
    );
    expect(order, stored);
    expect(disabled, {'/scenes/Mine'});
    // Listed again later: still first, still unchecked.
    final (again, stillDisabled) = reconcileScenes(
      listed: ['builtin:A', wall, '/scenes/Mine'],
      storedOrder: order,
      storedDisabled: disabled,
    );
    expect(again, stored);
    expect(stillDisabled, {'/scenes/Mine'});
  });

  test('new scenes come checked at the end', () {
    final (order, disabled) = reconcileScenes(
      listed: [wall, 'builtin:A', '/scenes/New'],
      storedOrder: [wall, 'builtin:A'],
      storedDisabled: ['builtin:A'],
    );
    expect(order, [wall, 'builtin:A', '/scenes/New']);
    expect(disabled, {'builtin:A'});
  });

  test('some listed scene stays checked, the cover wall first', () {
    // The only checked scene was deleted.
    final (_, disabled) = reconcileScenes(
      listed: ['builtin:A', wall],
      storedOrder: ['builtin:A', wall],
      storedDisabled: ['builtin:A', wall],
    );
    expect(disabled, {'builtin:A'});
    final (_, withoutWall) = reconcileScenes(
      listed: ['builtin:A', 'builtin:B'],
      storedOrder: ['builtin:B', 'builtin:A'],
      storedDisabled: ['builtin:A', 'builtin:B'],
    );
    expect(withoutWall, {'builtin:A'});
  });

  test('arranging the listed scenes leaves the others in place', () {
    final order = ['builtin:A', '/scenes/Gone', wall, 'builtin:B'];
    expect(
      placeArranged(order, ['builtin:B', 'builtin:A', wall]),
      ['builtin:B', '/scenes/Gone', 'builtin:A', wall],
    );
  });
}
