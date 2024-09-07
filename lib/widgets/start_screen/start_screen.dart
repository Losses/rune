import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';
import 'package:provider/provider.dart';

import '../../config/animation.dart';

import '../smooth_horizontal_scroll.dart';

import './start_group.dart';
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

  final _layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();
    summary = widget.fetchSummary();
    _pagingController.addPageRequestListener((cursor) async {
      await widget.fetchPage(_pagingController, cursor);

      Timer(Duration(milliseconds: gridAnimationDelay),
          () => _layoutManager.playAnimations());
    });
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
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
                    pagingController: _pagingController,
                    scrollDirection: Axis.horizontal,
                    scrollController: scrollController,
                    builderDelegate: PagedChildBuilderDelegate<Group<T>>(
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
        ));
  }

  @override
  void dispose() {
    super.dispose();

    _pagingController.dispose();
    _layoutManager.dispose();
  }
}
