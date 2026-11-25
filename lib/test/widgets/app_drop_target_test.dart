import 'package:desktop_drop/desktop_drop.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rune/widgets/router/app_drop_target.dart';

void main() {
  testWidgets(
    'drop boundary uses view DPR while child keeps effective metrics',
    (tester) async {
      tester.view.devicePixelRatio = 2;
      addTearDown(tester.view.resetDevicePixelRatio);
      await tester.pumpWidget(
        MediaQuery(
          data: const MediaQueryData(size: Size(400, 300), devicePixelRatio: 3),
          child: Directionality(
            textDirection: TextDirection.ltr,
            child: Transform.scale(
              scale: 1.5,
              alignment: Alignment.topLeft,
              child: const AppDropTarget(
                child: SizedBox(width: 200, height: 200),
              ),
            ),
          ),
        ),
      );
      final dropContext = tester.element(find.byType(DropTarget));
      final childContext = tester.element(find.byType(SizedBox));
      expect(MediaQuery.devicePixelRatioOf(dropContext), 2);
      expect(MediaQuery.devicePixelRatioOf(childContext), 3);
      final box = dropContext.findRenderObject()! as RenderBox;
      // Windows/Android native physical point is divided by system DPR by the
      // plugin, then globalToLocal removes the application transform.
      expect(
        box.globalToLocal(const Offset(300, 150) / 2),
        const Offset(100, 50),
      );
    },
  );
}
