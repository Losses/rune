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

  final Map<String, GlobalKey> itemKeys = {};

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

  Future<bool> scrollToGroup(String groupTitle) async {
    final key = itemKeys[groupTitle];
    if (key != null && key.currentContext != null) {
      final RenderBox box = key.currentContext!.findRenderObject() as RenderBox;
      final position = box.localToGlobal(Offset.zero).dx;
      await scrollController.animateTo(
        position,
        duration: const Duration(milliseconds: 300),
        curve: Curves.easeInOut,
      );
      return true;
    }

    if (isLastPage) return false;

    while (!isLastPage) {
      await _fetchDataAsync();
      if (await scrollToGroup(groupTitle)) return true;
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
      for (var item in newItems) {
        itemKeys[item.groupTitle] = GlobalKey();
      }
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
      itemKeys.clear();
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
            return LayoutBuilder(
              builder: (context, constraints) {
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
                      onFetchData: _fetchDataAsync,
                      hasReachedMax: isLastPage,
                      itemBuilder: (context, index) {
                        final item = items[index];
                        return StartGroup<InternalCollection>(
                          key: itemKeys[item.groupTitle]!,
                          groupIndex: index,
                          groupTitle: item.groupTitle,
                          items: item.items,
                          constraints: constraints,
                          onTitleTap: () {
                            showGroupListDialog(context);
                          },
                          itemBuilder: (context, item) =>
                              widget.itemBuilder(context, item, _reloadData),
                        );
                      },
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
