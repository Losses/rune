import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';

import '../../widgets/start_screen/managed_start_screen_item.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';

void main() {
  testWidgets('items built while a list reloads stay visible', (tester) async {
    final layout = StartScreenLayoutManager();
    addTearDown(layout.dispose);
    // A column of items; more of them come into view as the list scrolls.
    Widget items(int count) => ChangeNotifierProvider.value(
      value: layout,
      child: Directionality(
        textDirection: TextDirection.ltr,
        child: Column(
          children: [
            for (var i = 0; i < count; i++)
              ManagedStartScreenItem(
                key: ValueKey(i),
                groupId: 0,
                row: i,
                column: 0,
                width: 10,
                height: 10,
                child: Text('item $i'),
              ),
          ],
        ),
      ),
    );
    Future<void> animate() async {
      for (var i = 0; i < 60; i++) {
        await tester.pump(const Duration(milliseconds: 16));
      }
    }

    // The first load plays the entrance.
    await tester.pumpWidget(items(2));
    expect(find.text('item 0'), findsNothing);
    layout.playAnimations();
    await animate();
    expect(find.text('item 0'), findsOneWidget);
    expect(find.text('item 1'), findsOneWidget);

    final reload = Completer<void>();
    final replay = layout.replayAfter(reload.future);
    await tester.pumpWidget(items(3));
    expect(find.text('item 0'), findsOneWidget);
    expect(find.text('item 2'), findsOneWidget);

    // Items the reload brings in play the entrance.
    reload.complete();
    await replay;
    await tester.pumpWidget(items(4));
    expect(find.text('item 3'), findsNothing);
    await animate();
    for (var i = 0; i < 4; i++) {
      expect(find.text('item $i'), findsOneWidget);
    }
  });
}
