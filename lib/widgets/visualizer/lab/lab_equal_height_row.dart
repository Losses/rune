import 'dart:math' as math;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// A row of [children] side by side, each as wide as it wants of what the
/// ones before it leave, and all as tall as the tallest of them, up to
/// [maxHeight]. With [fill] the last child is as wide as the rest of the row.
class LabEqualHeightRow extends MultiChildRenderObjectWidget {
  const LabEqualHeightRow({
    super.key,
    required this.gap,
    required this.maxHeight,
    this.height,
    this.fill = false,
    required super.children,
  });

  /// The space between two children.
  final double gap;

  /// The tallest the children get; past it they are given this height.
  final double maxHeight;

  /// The height every child is given exactly, in one pass; none lays the
  /// children out twice, to find the tallest. It is held to the constraints.
  final double? height;

  /// Whether the row fills the width it is given: the last child is exactly
  /// as wide as what the others leave of it, rather than as wide as it wants.
  final bool fill;

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderLabEqualHeightRow(
        gap: gap,
        maxHeight: maxHeight,
        height: height,
        fill: fill,
      );

  @override
  void updateRenderObject(BuildContext context, RenderObject renderObject) {
    (renderObject as _RenderLabEqualHeightRow)
      ..gap = gap
      ..maxHeight = maxHeight
      ..height = height
      ..fill = fill;
  }
}

class _LabEqualHeightParentData extends ContainerBoxParentData<RenderBox> {}

class _RenderLabEqualHeightRow extends RenderBox
    with
        ContainerRenderObjectMixin<RenderBox, _LabEqualHeightParentData>,
        RenderBoxContainerDefaultsMixin<RenderBox, _LabEqualHeightParentData> {
  _RenderLabEqualHeightRow({
    required double gap,
    required double maxHeight,
    required double? height,
    required bool fill,
  }) : _gap = gap,
       _maxHeight = maxHeight,
       _height = height,
       _fill = fill;

  double _gap;
  set gap(double value) {
    if (value == _gap) return;
    _gap = value;
    markNeedsLayout();
  }

  double _maxHeight;
  set maxHeight(double value) {
    if (value == _maxHeight) return;
    _maxHeight = value;
    markNeedsLayout();
  }

  double? _height;
  set height(double? value) {
    if (value == _height) return;
    _height = value;
    markNeedsLayout();
  }

  bool _fill;
  set fill(bool value) {
    if (value == _fill) return;
    _fill = value;
    markNeedsLayout();
  }

  // The constraints of [child] at [x] from the start: no wider than what is
  // left of the row, and the last child of a row that fills all of it.
  BoxConstraints _at(
    RenderBox child,
    double x, {
    double minHeight = 0,
    required double maxHeight,
  }) {
    final left = math.max(0.0, constraints.maxWidth - x);
    final exact = _fill && child == lastChild && left.isFinite;
    return BoxConstraints(
      minWidth: exact ? left : 0,
      maxWidth: left,
      minHeight: minHeight,
      maxHeight: maxHeight,
    );
  }

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! _LabEqualHeightParentData) {
      child.parentData = _LabEqualHeightParentData();
    }
  }

  @override
  void performLayout() {
    final fixed = _height;
    var tallest = 0.0;
    if (fixed != null) {
      tallest = math.max(0.0, math.min(fixed, constraints.maxHeight));
    } else {
      final cap = math.max(0.0, math.min(_maxHeight, constraints.maxHeight));
      // First as tall as each wants, to find the tallest; then all as tall.
      var x = 0.0;
      for (var child = firstChild; child != null; child = childAfter(child)) {
        child.layout(_at(child, x, maxHeight: cap), parentUsesSize: true);
        tallest = math.max(tallest, child.size.height);
        x += child.size.width + _gap;
      }
    }
    var x = 0.0;
    for (var child = firstChild; child != null; child = childAfter(child)) {
      child.layout(
        _at(child, x, minHeight: tallest, maxHeight: tallest),
        parentUsesSize: true,
      );
      (child.parentData! as _LabEqualHeightParentData).offset = Offset(x, 0);
      x += child.size.width + _gap;
    }
    size = constraints.constrain(Size(childCount == 0 ? 0 : x - _gap, tallest));
  }

  @override
  void paint(PaintingContext context, Offset offset) =>
      defaultPaint(context, offset);

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      defaultHitTestChildren(result, position: position);
}
