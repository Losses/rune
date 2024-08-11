import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../widgets/smooth_horizontal_scroll.dart';

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

class StartGroup<T> extends StatelessWidget {
  final List<T> items;
  final String groupTitle;
  final int index;
  final Widget Function(BuildContext, T) itemBuilder;

  const StartGroup({
    super.key,
    required this.index,
    required this.groupTitle,
    required this.items,
    required this.itemBuilder,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Container(
      padding: const EdgeInsets.only(left: 16, right: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 4),
            child: Text(groupTitle, style: theme.typography.bodyLarge),
          ),
          Expanded(
            child: StartGroupItem<T>(
              cellSize: 120,
              gapSize: 4,
              items: items,
              itemBuilder: itemBuilder,
            ),
          ),
        ],
      ),
    );
  }
}

class StartGroupItem<T> extends StatelessWidget {
  final double cellSize;
  final double gapSize;
  final List<T> items;
  final Widget Function(BuildContext, T) itemBuilder;

  const StartGroupItem({
    super.key,
    required this.cellSize,
    required this.gapSize,
    required this.items,
    required this.itemBuilder,
  });

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final double containerHeight = constraints.maxHeight;
        final int rows = (containerHeight / (cellSize + gapSize)).floor();
        final int columns = (items.length / rows).ceil();

        final double finalHeight = rows * (cellSize + gapSize) - gapSize;
        final double finalWidth = columns * (cellSize + gapSize) - gapSize;

        return SizedBox(
          width: finalWidth,
          height: finalHeight,
          child: Wrap(
            spacing: gapSize,
            runSpacing: gapSize,
            children: items.map((item) {
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
