import 'package:flutter_test/flutter_test.dart';

import 'package:rune/providers/playback_controller.dart';
import 'package:rune/widgets/playback_controller/constants/controller_items.dart';

void main() {
  test('an entry the saved order lacks goes to the end, past the hidden', () {
    final ids = [for (final item in controllerItems) item.id];
    final added = ids.last;
    // An order saved before the last entry came: the first two swapped.
    final saved = [ids[1], ids[0], ...ids.sublist(2, ids.length - 1)];
    final ordered = [
      for (final entry in orderControllerEntries(controllerItems, saved))
        entry.id,
    ];
    expect(ordered.sublist(0, saved.length), saved);
    expect(ordered.last, added);
    expect(ordered.indexOf(added), greaterThan(ordered.indexOf('hidden')));
  });

  test('entries the saved order lacks keep their own order', () {
    final ids = [for (final item in controllerItems) item.id];
    final ordered = [
      for (final entry in orderControllerEntries(controllerItems, [ids.last]))
        entry.id,
    ];
    expect(ordered, [ids.last, ...ids.sublist(0, ids.length - 1)]);
  });
}
