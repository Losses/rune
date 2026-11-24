import 'dart:math' as math;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

/// Sections stacked top to bottom in columns, as many columns as the height
/// it is given needs: a section that would not fit under the ones before it
/// starts the next column. Every section is [columnWidth] wide, or with
/// [fitContent] as wide as the widest section asks to be, up to that. Given
/// no height limit it is one column.
///
/// The columns are narrower when they would not fit the width it is given,
/// down to [minColumnWidth]; their sections wrap, and may take more columns,
/// which are narrower again. Past that it is one column as wide as its room,
/// all its sections in it, which a vertical scroll view around it scrolls.
///
/// A section taller than the limit gets a column of its own, as tall as the
/// limit, and scrolls vertically inside it.
class LabSectionFlow extends MultiChildRenderObjectWidget {
  LabSectionFlow({
    super.key,
    required this.columnWidth,
    this.fitContent = false,
    this.minColumnWidth = 0,
    this.flowHeight,
    this.columnGap = 16,
    this.sectionGap = 10,
    required List<Widget> children,
  }) : super(children: [for (final child in children) _ScrollSection(child)]);

  /// The width of every column, or with [fitContent] the most it may be.
  final double columnWidth;

  /// Whether the columns are as wide as the widest section asks to be (its
  /// greatest intrinsic width), rather than [columnWidth]. A part wrapped in
  /// [LabFillWidth] asks for no width and takes what the others set.
  final bool fitContent;

  /// The narrowest the columns get to fit the width the flow is given.
  final double minColumnWidth;

  /// The height the sections flow down to, when it is not the height the
  /// flow is given: in a vertical scroll view, which gives none, the view's.
  final double? flowHeight;

  /// The space between two columns.
  final double columnGap;

  /// The space between two sections of a column.
  final double sectionGap;

  @override
  RenderObject createRenderObject(BuildContext context) => _RenderSectionFlow(
    columnWidth: columnWidth,
    fitContent: fitContent,
    minColumnWidth: minColumnWidth,
    flowHeight: flowHeight,
    columnGap: columnGap,
    sectionGap: sectionGap,
  );

  @override
  void updateRenderObject(BuildContext context, RenderObject renderObject) {
    (renderObject as _RenderSectionFlow)
      ..columnWidth = columnWidth
      ..fitContent = fitContent
      ..minColumnWidth = minColumnWidth
      ..flowHeight = flowHeight
      ..columnGap = columnGap
      ..sectionGap = sectionGap;
  }
}

/// A part of a section that asks for no width of its own and fills what it
/// is given: a list or a choice that ends where the widest row of buttons
/// does, in a [LabSectionFlow] that fits its content.
class LabFillWidth extends SingleChildRenderObjectWidget {
  const LabFillWidth({super.key, required Widget super.child});

  @override
  RenderObject createRenderObject(BuildContext context) => _RenderFillWidth();
}

class _RenderFillWidth extends RenderProxyBox {
  @override
  double computeMinIntrinsicWidth(double height) => 0;

  @override
  double computeMaxIntrinsicWidth(double height) => 0;
}

/// A section that takes the height of its content, up to what it is given,
/// and scrolls past that.
class _ScrollSection extends StatelessWidget {
  const _ScrollSection(this.child);

  final Widget child;

  @override
  Widget build(BuildContext context) =>
      SingleChildScrollView(primary: false, child: child);
}

class _SectionFlowParentData extends ContainerBoxParentData<RenderBox> {}

class _RenderSectionFlow extends RenderBox
    with
        ContainerRenderObjectMixin<RenderBox, _SectionFlowParentData>,
        RenderBoxContainerDefaultsMixin<RenderBox, _SectionFlowParentData> {
  _RenderSectionFlow({
    required double columnWidth,
    required bool fitContent,
    required double minColumnWidth,
    required double? flowHeight,
    required double columnGap,
    required double sectionGap,
  }) : _columnWidth = columnWidth,
       _fitContent = fitContent,
       _minColumnWidth = minColumnWidth,
       _flowHeight = flowHeight,
       _columnGap = columnGap,
       _sectionGap = sectionGap;

  double _columnWidth;
  set columnWidth(double value) {
    if (value == _columnWidth) return;
    _columnWidth = value;
    markNeedsLayout();
  }

  bool _fitContent;
  set fitContent(bool value) {
    if (value == _fitContent) return;
    _fitContent = value;
    markNeedsLayout();
  }

  double _minColumnWidth;
  set minColumnWidth(double value) {
    if (value == _minColumnWidth) return;
    _minColumnWidth = value;
    markNeedsLayout();
  }

  double? _flowHeight;
  set flowHeight(double? value) {
    if (value == _flowHeight) return;
    _flowHeight = value;
    markNeedsLayout();
  }

  double _columnGap;
  set columnGap(double value) {
    if (value == _columnGap) return;
    _columnGap = value;
    markNeedsLayout();
  }

  double _sectionGap;
  set sectionGap(double value) {
    if (value == _sectionGap) return;
    _sectionGap = value;
    markNeedsLayout();
  }

  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! _SectionFlowParentData) {
      child.parentData = _SectionFlowParentData();
    }
  }

  @override
  void performLayout() {
    var columnWidth = _columnWidth;
    if (_fitContent) {
      var widest = 0.0;
      for (var child = firstChild; child != null; child = childAfter(child)) {
        widest = math.max(widest, child.getMaxIntrinsicWidth(double.infinity));
      }
      columnWidth = math.min(columnWidth, widest.ceilToDouble());
    }
    final room = constraints.maxWidth;
    final fitted = columnWidth;
    columnWidth = math.max(_minColumnWidth, math.min(columnWidth, room));
    final limit = _flowHeight ?? constraints.maxHeight;

    var (columns, tallest) = _flow(columnWidth, limit);
    // Columns that do not fit share the room; narrower, their sections may
    // take more columns, which share it again. The width only falls, and
    // the columns only grow in number, up to one a section.
    while (_widthOf(columns, columnWidth) > room) {
      final share = ((room - (columns - 1) * _columnGap) / columns)
          .floorToDouble();
      final narrower = math.max(_minColumnWidth, share);
      if (narrower >= columnWidth) break;
      columnWidth = narrower;
      (columns, tallest) = _flow(columnWidth, limit);
    }
    if (_widthOf(columns, columnWidth) > room) {
      // One column, as tall as its sections, to scroll.
      columnWidth = math.min(fitted, room);
      (columns, tallest) = _flow(columnWidth, double.infinity);
    }

    size = constraints.constrain(Size(_widthOf(columns, columnWidth), tallest));
  }

  double _widthOf(int columns, double columnWidth) =>
      columns == 0 ? 0 : columns * columnWidth + (columns - 1) * _columnGap;

  // Lays the sections out in columns [columnWidth] wide, down to [limit],
  // and gives the number of columns and the height of the tallest.
  (int, double) _flow(double columnWidth, double limit) {
    final width = BoxConstraints.tightFor(width: columnWidth);

    var column = 0;
    var y = 0.0;
    var tallest = 0.0;
    var empty = true;
    for (var child = firstChild; child != null; child = childAfter(child)) {
      // First as tall as it wants, to see where it goes.
      child.layout(width, parentUsesSize: true);
      var h = child.size.height;
      if (!empty && y + _sectionGap + h > limit) {
        column++;
        y = 0;
        empty = true;
      }
      if (h > limit) {
        // Alone in a column that is as tall as the limit; it scrolls.
        child.layout(
          BoxConstraints.tightFor(width: columnWidth, height: limit),
          parentUsesSize: true,
        );
        h = limit;
      }
      if (!empty) y += _sectionGap;
      (child.parentData! as _SectionFlowParentData).offset = Offset(
        column * (columnWidth + _columnGap),
        y,
      );
      y += h;
      empty = false;
      tallest = math.max(tallest, y);
    }

    final columns = empty && column == 0 ? 0 : column + 1;
    return (columns, tallest);
  }

  @override
  void paint(PaintingContext context, Offset offset) =>
      defaultPaint(context, offset);

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      defaultHitTestChildren(result, position: position);
}
