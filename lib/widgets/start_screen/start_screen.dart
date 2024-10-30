import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';

import '../infinite_list_loading.dart';
import '../smooth_horizontal_scroll.dart';

import 'utils/group.dart';
import 'utils/internal_collection.dart';
import 'providers/start_screen_layout_manager.dart';

import 'start_group.dart';
import 'start_group_implementation.dart';

class StartScreen extends StatelessWidget {
  final Future<List<Group<InternalCollection>>> Function() fetchSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(int) fetchPage;
  final Widget Function(BuildContext, InternalCollection, VoidCallback)
      itemBuilder;
  final bool userGenerated;

  const StartScreen({
    super.key,
    required this.fetchSummary,
    required this.fetchPage,
    required this.itemBuilder,
    required this.userGenerated,
  });

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(builder: (context, constraints) {
      return StartScreenImplementation(
        fetchSummary: fetchSummary,
        fetchPage: fetchPage,
        itemBuilder: itemBuilder,
        userGenerated: userGenerated,
        constraints: constraints,
      );
    });
  }
}

class StartScreenImplementation extends StatefulWidget {
  final Future<List<Group<InternalCollection>>> Function() fetchSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(int) fetchPage;
  final Widget Function(BuildContext, InternalCollection, VoidCallback)
      itemBuilder;
  final bool userGenerated;

  final BoxConstraints constraints;

  const StartScreenImplementation({
    super.key,
    required this.fetchSummary,
    required this.fetchPage,
    required this.itemBuilder,
    required this.userGenerated,
    required this.constraints,
  });

  @override
  StartScreenImplementationState createState() =>
      StartScreenImplementationState();
}

class StartScreenImplementationState extends State<StartScreenImplementation>
    with SingleTickerProviderStateMixin {
  late Future<List<Group<InternalCollection>>> summary;
  final layoutManager = StartScreenLayoutManager();
  late final scrollController = SmoothScrollController(vsync: this);

  List<Group<InternalCollection>> items = [];
  bool isLoading = false;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
  }

  @override
  void dispose() {
    scrollController.dispose();
    layoutManager.dispose();
    super.dispose();
  }

  Future<void> scrollToGroup(String groupTitle) async {
    // Step 1: Check if the group already exists in the loaded items.
    while (!isLastPage) {
      final index = items.indexWhere((group) => group.groupTitle == groupTitle);

      // If found, calculate the scroll position.
      if (index != -1) {
        double scrollPosition = 0.0;

        // Step 5: Calculate the scroll position for the target group.
        for (int i = 0; i < index; i++) {
          final group = items[i];
          final dimensions =
              StartGroupImplementation.defaultDimensionCalculator(
            widget.constraints.maxHeight,
            defaultCellSize,
            4,
            group.items,
          );

          final (groupWidth, _) = StartGroupImplementation.finalSizeCalculator(
            dimensions,
            defaultCellSize,
            4,
          );

          scrollPosition += groupWidth + defaultGapSize + 32;
        }

        // Step 6: Scroll to the calculated position.
        scrollController.scrollTo(
          scrollPosition,
        );
        return;
      }

      // Step 2: If not found, load the next page.
      await _fetchDataAsync();
    }

    // Step 3: If reached here, it means we didn't find the group and reached the last page.
    // Silent return as specified.
  }

  Future<void> _fetchDataAsync() async {
    if (isLoading || isLastPage) return;

    setState(() {
      initialized = true;
      isLoading = true;
    });

    final thisCursor = cursor;
    cursor += 1;
    final (newItems, newIsLastPage) = await widget.fetchPage(thisCursor);

    setState(() {
      isLoading = false;
      isLastPage = newIsLastPage;
      items.addAll(newItems);
    });

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => layoutManager.playAnimations(),
    );
  }

  void _reloadData() {
    setState(() {
      cursor = 0;
      items = [];
      isLastPage = false;
    });
    _fetchDataAsync();
  }

  void showGroupListDialog(BuildContext context) async {
    await showDialog<void>(
      context: context,
      builder: (context) => FutureBuilder<List<Group<InternalCollection>>>(
        future: summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return Container();
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else {
            return ContentDialog(
              constraints: const BoxConstraints(maxWidth: 320),
              content: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.end,
                mainAxisSize: MainAxisSize.min,
                children: [
                  Wrap(
                    spacing: 4,
                    runSpacing: 4,
                    children: snapshot.data!
                        .map(
                          (x) => ConstrainedBox(
                            constraints: const BoxConstraints(maxWidth: 40),
                            child: AspectRatio(
                              aspectRatio: 1,
                              child: Button(
                                child: Text(x.groupTitle),
                                onPressed: () {
                                  Navigator.pop(context);
                                  scrollToGroup(x.groupTitle);
                                },
                              ),
                            ),
                          ),
                        )
                        .toList(),
                  ),
                  const SizedBox(height: 24),
                  Button(
                    child: const Text('Cancel'),
                    onPressed: () => Navigator.pop(context),
                  ),
                ],
              ),
            );
          }
        },
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return Container();
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else {
            return SmoothHorizontalScroll(
              controller: scrollController,
              builder: (context, smoothScrollController) {
                return InfiniteList(
                  itemCount: items.length,
                  scrollDirection: Axis.horizontal,
                  scrollController: smoothScrollController,
                  loadingBuilder: (context) => const InfiniteListLoading(),
                  centerLoading: true,
                  centerEmpty: true,
                  isLoading: isLoading,
                  emptyBuilder: (context) => Center(
                    child: initialized
                        ? NoItems(
                            title: "No collection found",
                            hasRecommendation: false,
                            reloadData: _reloadData,
                            userGenerated: widget.userGenerated,
                          )
                        : Container(),
                  ),
                  onFetchData: _fetchDataAsync,
                  hasReachedMax: isLastPage,
                  itemBuilder: (context, index) {
                    final item = items[index];
                    return StartGroup<InternalCollection>(
                      key: ValueKey(item.groupTitle),
                      groupIndex: index,
                      groupTitle: item.groupTitle,
                      items: item.items,
                      constraints: widget.constraints,
                      onTitleTap: () {
                        if (!widget.userGenerated) {
                          showGroupListDialog(context);
                        }
                      },
                      itemBuilder: (context, item) =>
                          widget.itemBuilder(context, item, _reloadData),
                    );
                  },
                );
              },
            );
          }
        },
      ),
    );
  }
}
