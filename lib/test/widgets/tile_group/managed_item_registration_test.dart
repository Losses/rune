import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import 'package:rune/widgets/start_screen/managed_start_screen_item.dart';
import 'package:rune/widgets/start_screen/providers/start_screen_layout_manager.dart';
import 'package:rune/widgets/turntile/managed_turntile_screen_item.dart';

// A retained tile that moves or changes its span must tell the layout manager,
// or the entrance animation of later groups starts from stale distances.

Widget _host(StartScreenLayoutManager manager, Widget child) {
  return Directionality(
    textDirection: TextDirection.ltr,
    child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: manager,
      child: child,
    ),
  );
}

Widget _start(int column, int columns) => ManagedStartScreenItem(
  groupId: 0,
  row: 0,
  column: column,
  columns: columns,
  width: 10,
  height: 10,
  child: const SizedBox.expand(key: ValueKey('tile')),
);

Widget _turntile(int column, int columns) => ManagedTurntileScreenItem(
  groupId: 0,
  row: 0,
  column: column,
  columns: columns,
  child: const SizedBox.expand(key: ValueKey('tile')),
);

void main() {
  for (final (name, build) in [('start', _start), ('turntile', _turntile)]) {
    testWidgets('A $name tile registers again when its place changes', (
      tester,
    ) async {
      final manager = StartScreenLayoutManager();
      await tester.pumpWidget(_host(manager, build(0, 1)));
      expect(manager.getItem(0, 0, 0)?.columns, 1);

      await tester.pumpWidget(_host(manager, build(2, 3)));

      expect(manager.getItem(0, 0, 0), isNull, reason: 'old place is free');
      expect(manager.getItem(0, 0, 2)?.columns, 3);
    });

    testWidgets('A $name tile that is shown stays shown when it moves', (
      tester,
    ) async {
      final manager = StartScreenLayoutManager();
      await tester.pumpWidget(_host(manager, build(0, 1)));
      manager.playAnimations();
      await tester.pump(const Duration(milliseconds: 100));
      await tester.pump(const Duration(seconds: 2));
      expect(find.byKey(const ValueKey('tile')), findsOneWidget);

      await tester.pumpWidget(_host(manager, build(1, 1)));
      await tester.pump();

      expect(find.byKey(const ValueKey('tile')), findsOneWidget);
      manager.cleanup();
    });
  }
}
