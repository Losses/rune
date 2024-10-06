import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../utils/query_list.dart';
import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../../messages/collection.pbserver.dart';

import './start_group.dart';
import '../smooth_horizontal_scroll.dart';

import './providers/start_screen_layout_manager.dart';

class Group<T> {
  final String groupTitle;
  final List<T> items;

  Group({
    required this.groupTitle,
    required this.items,
  });
}

class InternalCollection {
  final int id;
  final String name;
  final QueryList queries;
  final CollectionType collectionType;
  final Map<int, String> coverArtMap;

  InternalCollection({
    required this.id,
    required this.name,
    required this.queries,
    required this.collectionType,
    required this.coverArtMap,
  });

  static InternalCollection fromRawCollection(Collection x) {
    return InternalCollection(
      id: x.id,
      name: x.name,
      queries: QueryList.fromMixQuery(x.queries),
      collectionType: x.collectionType,
      coverArtMap: x.coverArtMap,
    );
  }
}

class StartScreen extends StatefulWidget {
  final Future<List<Group<InternalCollection>>> Function() fetchSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(int) fetchPage;
  final Widget Function(BuildContext, InternalCollection, VoidCallback) itemBuilder;
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

class StartScreenState extends State<StartScreen> {
  late Future<List<Group<InternalCollection>>> summary;

  final layoutManager = StartScreenLayoutManager();

  List<Group<InternalCollection>> items = [];

  bool isLoading = false;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  void _fetchData() async {
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

  void _reloadData() async {
    cursor = 0;
    items = [];
    _fetchData();
  }

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
  }

  @override
  void dispose() {
    super.dispose();
    layoutManager.dispose();
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
              builder: (context, scrollController) {
                return InfiniteList(
                  itemCount: items.length,
                  scrollDirection: Axis.horizontal,
                  scrollController: scrollController,
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
                      itemBuilder: (context, item) => widget.itemBuilder(context, item, _reloadData),
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
