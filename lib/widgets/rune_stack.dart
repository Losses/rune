import 'package:flutter/rendering.dart';
import 'package:fluent_ui/fluent_ui.dart';

class RuneStack extends Stack {
  const RuneStack({super.key, super.children, super.alignment, super.fit});

  @override
  CustomRenderStack createRenderObject(BuildContext context) {
    return CustomRenderStack(
      alignment: alignment,
      textDirection: textDirection ?? Directionality.of(context),
      fit: fit,
    );
  }
}

class CustomRenderStack extends RenderStack {
  CustomRenderStack({
    super.alignment,
    super.textDirection,
    super.fit,
    overflow,
  });

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) {
    var stackHit = false;

    final children = getChildrenAsList();

    for (final child in children) {
      final StackParentData childParentData =
          child.parentData as StackParentData;

      final childHit = result.addWithPaintOffset(
        offset: childParentData.offset,
        position: position,
        hitTest: (BoxHitTestResult result, Offset transformed) {
          assert(transformed == position - childParentData.offset);
          return child.hitTest(result, position: transformed);
        },
      );

      if (childHit) stackHit = true;
    }

    return stackHit;
  }
}
