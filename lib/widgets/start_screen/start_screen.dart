import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../config/animation.dart';
import '../../widgets/no_items.dart';

import '../smooth_horizontal_scroll.dart';

import './start_group.dart';
import './utils/group.dart';
import './utils/internal_collection.dart';
import './providers/start_screen_layout_manager.dart';

class StartScreen extends StatefulWidget {
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
  StartScreenState createState() => StartScreenState();
}

class StartScreenState extends State<StartScreen>
    with SingleTickerProviderStateMixin {
  late Future<List<Group<InternalCollection>>> summary;
  final layoutManager = StartScreenLayoutManager();
  late final scrollController = SmoothScrollController(vsync: this);

  List<Group<InternalCollection>> items = [];
  bool isLoading = false;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  int? _findGroupIndex(String groupTitle) {
    for (int i = 0; i < items.length; i++) {
      if (items[i].groupTitle == groupTitle) {
        return i;
      }
    }
    return null;
  }

  Future<bool> scrollToGroup(String groupTitle) async {
    int? index = _findGroupIndex(groupTitle);

    if (index != null) {
      await scrollController.animateTo(
        index * 300.0,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );
      return true;
    }

    if (isLastPage) {
      return false;
    }

    while (!isLastPage) {
      await _fetchDataAsync();
      index = _findGroupIndex(groupTitle);
      if (index != null) {
        await scrollController.animateTo(
          index * 300.0,
          duration: const Duration(milliseconds: 300),
          curve: Curves.easeInOut,
        );
        return true;
      }
    }

    return false;
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

  void _fetchData() {
    _fetchDataAsync();
  }

  void _reloadData() async {
    setState(() {
      cursor = 0;
      items = [];
      isLastPage = false;
    });
    _fetchData();
  }

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
                  loadingBuilder: (context) => const ProgressRing(),
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
                  onFetchData: _fetchData,
                  hasReachedMax: isLastPage,
                  itemBuilder: (context, index) {
                    final item = items[index];
                    return StartGroup<InternalCollection>(
                      key: Key(item.groupTitle),
                      groupIndex: index,
                      groupTitle: item.groupTitle,
                      items: item.items,
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
