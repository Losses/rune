import 'package:flutter/rendering.dart';
import 'package:fluent_ui/fluent_ui.dart';

class CustomStackParentData extends StackParentData {
  bool blockHitTest = false;
}

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

class BlockHitTestStack extends ParentDataWidget<CustomStackParentData> {
  final bool blockHitTest;

  const BlockHitTestStack({
    super.key,
    required super.child,
    this.blockHitTest = true,
  });

  @override
  void applyParentData(RenderObject renderObject) {
    assert(renderObject.parentData is CustomStackParentData);
    final parentData = renderObject.parentData as CustomStackParentData;
    if (parentData.blockHitTest != blockHitTest) {
      parentData.blockHitTest = blockHitTest;
      final targetParent = renderObject.parent;
      if (targetParent is RenderObject) {
        targetParent.markNeedsLayout();
      }
    }
  }

  @override
  Type get debugTypicalAncestorWidgetClass => RuneStack;
}

class CustomRenderStack extends RenderStack {
  CustomRenderStack({
    super.alignment,
    super.textDirection,
    super.fit,
    overflow,
  });

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! CustomStackParentData) {
      child.parentData = CustomStackParentData();
    }
  }

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) {
    // Get the list of child components and reverse the order, so the last added child is tested first
    final children = getChildrenAsList().reversed;

    for (final child in children) {
      final childParentData = child.parentData as CustomStackParentData;

      if (!child.size.contains(position - childParentData.offset)) {
        continue; // If the click position is not within the child's bounds, skip this child
      }

      final childHit = result.addWithPaintOffset(
        offset: childParentData.offset,
        position: position,
        hitTest: (BoxHitTestResult result, Offset transformed) {
          assert(transformed == position - childParentData.offset);
          return child.hitTest(result, position: transformed);
        },
      );

      // If hit and needs to block, immediately return true and stop testing other children
      if (childHit && childParentData.blockHitTest) {
        return true;
      }

      // If hit but does not need to block, continue testing other children
      if (childHit) {
        continue;
      }
    }

    return false;
  }
}
