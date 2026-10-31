import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:provider/provider.dart';
import 'package:rune/providers/app_scale.dart';
import 'package:rune/widgets/app_viewport.dart';
import 'package:rune/widgets/visualizer/visualizer_render_size.dart';
import 'package:rune/utils/native_window_metrics.dart';

void main() {
  testWidgets(
    'viewport scaling preserves texture pixels and native control clearance',
    (tester) async {
      final scale = AppScaleProvider(
        read: () async => 100,
        write: (_) async {},
      );
      (int, int)? texture;
      double? nativeClearance;
      await tester.pumpWidget(
        ChangeNotifierProvider.value(
          value: scale,
          child: Directionality(
            textDirection: TextDirection.ltr,
            child: AppViewport(
              child: LayoutBuilder(
                builder: (context, constraints) {
                  texture = visualizerRenderSize(
                    constraints.biggest,
                    MediaQuery.devicePixelRatioOf(context),
                  );
                  nativeClearance = nativeWindowLogicalToLocal(context, 24);
                  return const SizedBox.expand();
                },
              ),
            ),
          ),
        ),
      );
      final original = texture;
      for (final percent in AppScaleProvider.supportedPercentages) {
        await scale.setPercentage(percent);
        await tester.pump();
        expect(
          texture,
          original,
          reason: 'UI zoom must not reduce physical texture resolution',
        );
        expect(nativeClearance! * scale.scaleFactor, closeTo(24, .00001));
      }
      await tester.pumpWidget(const SizedBox());
      scale.dispose();
    },
  );
  test(
    'texture allocation stays aligned and bounded for extreme aspect ratios',
    () {
      for (final size in [
        const Size(1, 10000),
        const Size(10000, 1),
        const Size(7680, 4320),
      ]) {
        final result = visualizerRenderSize(size, 2);
        expect(result.$1 % 64, 0);
        expect(result.$2 % 2, 0);
        expect(result.$1, inInclusiveRange(64, 1920));
        expect(result.$2, inInclusiveRange(2, 1080));
      }
      expect(() => visualizerRenderSize(Size.zero, 1), throwsArgumentError);
      expect(
        () => visualizerRenderSize(const Size(100, 100), double.nan),
        throwsArgumentError,
      );
    },
  );
}
