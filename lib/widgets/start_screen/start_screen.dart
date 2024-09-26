import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../tile/cover_art_manager.dart';

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

class StartScreen<T> extends StatefulWidget {
  final Future<List<Group<T>>> Function() fetchSummary;
  final Future<void> Function(PagingController<int, Group<T>>, int) fetchPage;
  final Widget Function(BuildContext, T) itemBuilder;
  final PagingController<int, Group<T>> pagingController;
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
  StartScreenState<T> createState() => StartScreenState<T>();
}

class StartScreenState<T> extends State<StartScreen<T>> {
  late Future<List<Group<T>>> summary;

  final _layoutManager = StartScreenLayoutManager();
  final _coverArtManager = CoverArtManager();

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
    widget.pagingController.addPageRequestListener((cursor) async {
      await widget.fetchPage(widget.pagingController, cursor);

      Timer(
        Duration(milliseconds: gridAnimationDelay),
        () => _layoutManager.playAnimations(),
      );
    });
  }

  @override
  void dispose() {
    super.dispose();
    _layoutManager.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<CoverArtManager>.value(
      value: _coverArtManager,
      child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: FutureBuilder<List<Group<T>>>(
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
                    pagingController: widget.pagingController,
                    scrollDirection: Axis.horizontal,
                    scrollController: scrollController,
                    builderDelegate: PagedChildBuilderDelegate<Group<T>>(
                      noItemsFoundIndicatorBuilder: (context) {
                        return NoItems(
                          title: "No collection found",
                          hasRecommendation: false,
                          pagingController: widget.pagingController,
                          userGenerated: widget.userGenerated,
                        );
                      },
                      itemBuilder: (context, item, index) => StartGroup<T>(
                        key: ValueKey(item.groupTitle),
                        groupIndex: index,
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
        ),
      ),
    );
  }
}
