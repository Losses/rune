import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../smooth_horizontal_scroll.dart';

import './normal_layout.dart';
import './stacked_layout.dart';

class Group<T> {
  final String groupTitle;
  final List<T> items;

  Group({
    required this.groupTitle,
    required this.items,
  });
}

class StartScreen<T> extends StatefulWidget {
  final Future<List<Group<T>>> Function() fetchSummary;
  final Future<void> Function(PagingController<int, Group<T>>, int) fetchPage;
  final Widget Function(BuildContext, T) itemBuilder;

  const StartScreen({
    super.key,
    required this.fetchSummary,
    required this.fetchPage,
    required this.itemBuilder,
  });

  @override
  StartScreenState<T> createState() => StartScreenState<T>();
}

class StartScreenState<T> extends State<StartScreen<T>> {
  final PagingController<int, Group<T>> _pagingController =
      PagingController(firstPageKey: 0);

  late Future<List<Group<T>>> summary;

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
    _pagingController.addPageRequestListener((cursor) {
      widget.fetchPage(_pagingController, cursor);
    });
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<Group<T>>>(
      future: summary,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text('Error: ${snapshot.error}'));
        } else {
          return SizedBox(
            width: MediaQuery.of(context).size.width,
            child: SmoothHorizontalScroll(
              builder: (context, scrollController) =>
                  PagedListView<int, Group<T>>(
                pagingController: _pagingController,
                scrollDirection: Axis.horizontal,
                scrollController: scrollController,
                builderDelegate: PagedChildBuilderDelegate<Group<T>>(
                  itemBuilder: (context, item, index) => StartGroup<T>(
                    index: index,
                    groupTitle: item.groupTitle,
                    items: item.items,
                    itemBuilder: widget.itemBuilder,
                  ),
                ),
              ),
            ),
          );
        }
      },
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}

enum StartGroupGridLayoutVariation { initial, square }

enum StartGroupGroupLayoutVariation { normal, stacked }

class StartGroup<T> extends StatelessWidget {
  final List<T> items;
  final String groupTitle;
  final int index;
  final Widget Function(BuildContext, T) itemBuilder;
  final StartGroupGridLayoutVariation gridLayoutVariation;
  final StartGroupGroupLayoutVariation groupLayoutVariation;

  final double gapSize;
  final VoidCallback? onTitleTap;

  const StartGroup({
    super.key,
    required this.index,
    required this.groupTitle,
    required this.items,
    required this.itemBuilder,
    this.gapSize = 4,
    this.onTitleTap,
    this.gridLayoutVariation = StartGroupGridLayoutVariation.initial,
    this.groupLayoutVariation = StartGroupGroupLayoutVariation.normal,
  });

  @override
  Widget build(BuildContext context) {
    return _buildStartGroupLayout(context, _buildStartGroupItems());
  }

  Widget _buildStartGroupLayout(BuildContext context, Widget child) {
    switch (groupLayoutVariation) {
      case StartGroupGroupLayoutVariation.stacked:
        return StartGroupStackedLayout(
            groupTitle: groupTitle, onTitleTap: onTitleTap, child: child);
      case StartGroupGroupLayoutVariation.normal:
      default:
        return StartGroupNormalLayout(
            groupTitle: groupTitle, onTitleTap: onTitleTap, child: child);
    }
  }

  Widget _buildStartGroupItems() {
    switch (gridLayoutVariation) {
      case StartGroupGridLayoutVariation.square:
        return StartGroupItems<T>.square(
          cellSize: 120,
          gapSize: gapSize,
          items: items,
          itemBuilder: itemBuilder,
        );
      case StartGroupGridLayoutVariation.initial:
      default:
        return StartGroupItems<T>(
          cellSize: 120,
          gapSize: gapSize,
          items: items,
          itemBuilder: itemBuilder,
        );
    }
  }
}

class Dimensions<T> {
  final int rows;
  final int columns;
  final int count;

  Dimensions({
    required this.rows,
    required this.columns,
    required this.count,
  });

  @override
  String toString() => "Dimensions($rows x $columns, count: $count)";
}

class StartGroupItems<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final Widget Function(BuildContext, T) itemBuilder;

  final Dimensions Function(double, double, double, List<T>)
      dimensionCalculator;

  const StartGroupItems({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.itemBuilder,
    Dimensions Function(double, double, double, List<T>)? dimensionCalculator,
  }) : dimensionCalculator = dimensionCalculator ?? _defaultDimensionCalculator;

  const StartGroupItems.square({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.itemBuilder,
  }) : dimensionCalculator = _squareDimensionCalculator;

  static Dimensions _defaultDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = (containerHeight / (cellSize + gapSize)).floor();
    final int columns = (items.length / rows).ceil();
    return Dimensions(rows: rows, columns: columns, count: items.length);
  }

  static Dimensions _squareDimensionCalculator(
      double containerHeight, double cellSize, double gapSize, List items) {
    final int rows = (containerHeight / (cellSize + gapSize)).floor();
    final int maxItems =
        min(pow(sqrt(items.length).floor(), 2).floor(), pow(rows, 2).floor());
    final int columns = min((maxItems / rows).ceil(), rows);
    return Dimensions(rows: rows, columns: columns, count: maxItems);
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final Dimensions dimensions = dimensionCalculator(
            constraints.maxHeight, cellSize, gapSize, items);

        final double finalHeight =
            dimensions.rows * (cellSize + gapSize) - gapSize;
        final double finalWidth =
            dimensions.columns * (cellSize + gapSize) - gapSize;

        return SizedBox(
          width: finalWidth,
          height: finalHeight,
          child: Wrap(
            spacing: gapSize,
            runSpacing: gapSize,
            children: items.take(dimensions.count).map((item) {
              return SizedBox(
                width: cellSize,
                height: cellSize,
                child: itemBuilder(context, item),
              );
            }).toList(),
          ),
        );
      },
    );
  }
}
