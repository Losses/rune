import 'dart:async';

import 'package:player/messages/collection.pbserver.dart';
import 'package:player/utils/query_list.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../config/animation.dart';
import '../../widgets/no_items.dart';

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

  InternalCollection({
    required this.id,
    required this.name,
    required this.queries,
    required this.collectionType,
  });

  static InternalCollection fromRawCollection(Collection x) {
    return InternalCollection(
      id: x.id,
      name: x.name,
      queries: QueryList.fromMixQuery(x.queries),
      collectionType: x.collectionType,
    );
  }
}

class StartScreen extends StatefulWidget {
  final Future<List<Group<InternalCollection>>> Function() fetchSummary;
  final Future<void> Function(
      PagingController<int, Group<InternalCollection>>, int) fetchPage;
  final Widget Function(BuildContext, InternalCollection) itemBuilder;
  final PagingController<int, Group<InternalCollection>> pagingController;
  final bool userGenerated;

  const StartScreen({
    super.key,
    required this.fetchSummary,
    required this.fetchPage,
    required this.itemBuilder,
    required this.pagingController,
    required this.userGenerated,
  });

  @override
  StartScreenState createState() => StartScreenState();
}

class StartScreenState extends State<StartScreen> {
  late Future<List<Group<InternalCollection>>> summary;

  final layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
    widget.pagingController.addPageRequestListener((cursor) async {
      await widget.fetchPage(widget.pagingController, cursor);

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => layoutManager.playAnimations(),
      );
    });
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
            return SizedBox(
              width: MediaQuery.of(context).size.width,
              child: SmoothHorizontalScroll(
                builder: (context, scrollController) {
                  return PagedListView<int, Group<InternalCollection>>(
                    pagingController: widget.pagingController,
                    scrollDirection: Axis.horizontal,
                    scrollController: scrollController,
                    builderDelegate:
                        PagedChildBuilderDelegate<Group<InternalCollection>>(
                      noItemsFoundIndicatorBuilder: (context) {
                        return NoItems(
                          title: "No collection found",
                          hasRecommendation: false,
                          pagingController: widget.pagingController,
                          userGenerated: widget.userGenerated,
                        );
                      },
                      itemBuilder: (context, item, index) {
                        return StartGroup<InternalCollection>(
                          key: ValueKey(item.groupTitle),
                          groupIndex: index,
                          groupTitle: item.groupTitle,
                          items: item.items,
                          itemBuilder: widget.itemBuilder,
                        );
                      },
                    ),
                  );
                },
              ),
            );
          }
        },
      ),
    );
  }
}
