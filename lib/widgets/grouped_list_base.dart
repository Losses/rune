import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../widgets/start_screen.dart';

abstract class GroupedListBase<T, S> extends StatefulWidget {
  const GroupedListBase({super.key});

  @override
  GroupedListBaseState<T, S> createState();
}

abstract class GroupedListBaseState<T, S> extends State<GroupedListBase<T, S>> {
  static const _pageSize = 3;

  final PagingController<int, Group<T>> _pagingController =
      PagingController(firstPageKey: 0);

  late Future<List<Group<S>>> summary;

  Future<List<Group<T>>> fetchSummary();

  Future<void> fetchPage(
    PagingController<int, Group<T>> controller,
    int cursor,
  ) async {
    try {
      final summaries = await fetchSummary();

      final startIndex = cursor * _pageSize;
      final endIndex = (cursor + 1) * _pageSize;

      if (startIndex >= summaries.length) {
        controller.appendLastPage([]);
        return;
      }

      final currentPageSummaries = summaries.sublist(
        startIndex,
        endIndex > summaries.length ? summaries.length : endIndex,
      );

      final groupTitles =
          currentPageSummaries.map((summary) => summary.groupTitle).toList();

      final groups = await fetchGroups(groupTitles);

      final isLastPage = endIndex >= summaries.length;
      if (isLastPage) {
        controller.appendLastPage(groups);
      } else {
        controller.appendPage(groups, cursor + 1);
      }
    } catch (error) {
      controller.error = error;
    }
  }

  Future<List<Group<T>>> fetchGroups(List<String> groupTitles);

  Widget itemBuilder(BuildContext context, T item);

  @override
  Widget build(BuildContext context) {
    return StartScreen<T>(
      fetchSummary: fetchSummary,
      fetchPage: fetchPage,
      itemBuilder: itemBuilder,
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}
