import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../widgets/smooth_horizontal_scroll.dart';

void main() {
  testWidgets(
    'a list rebuilt during a smooth scroll scrolls on from where it is',
    (tester) async {
      late SmoothScrollController controller;
      var showList = true;
      late StateSetter setShowList;

      await tester.pumpWidget(
        Directionality(
          textDirection: TextDirection.ltr,
          child: StatefulBuilder(
            builder: (context, setState) {
              setShowList = setState;
              return SmoothHorizontalScroll(
                builder: (context, scrollController) {
                  controller = scrollController as SmoothScrollController;
                  if (!showList) return const SizedBox();
                  return ListView.builder(
                    scrollDirection: Axis.horizontal,
                    controller: scrollController,
                    itemCount: 100,
                    itemBuilder: (context, index) =>
                        SizedBox(width: 100, child: Text('$index')),
                  );
                },
              );
            },
          ),
        ),
      );

      controller.smoothScrollBy(3000);
      await tester.pump(const Duration(milliseconds: 16));
      await tester.pump(const Duration(milliseconds: 16));
      final cutShortAt = controller.offset;
      expect(cutShortAt, greaterThan(0));
      expect(cutShortAt, lessThan(3000));

      // The list goes away mid-scroll, and the animation ends with it.
      setShowList(() => showList = false);
      await tester.pumpAndSettle();
      setShowList(() => showList = true);
      await tester.pumpAndSettle();
      final start = controller.offset;

      controller.smoothScrollBy(50);
      await tester.pumpAndSettle();
      expect(controller.offset, closeTo(start + 50, 1));
    },
  );
}
