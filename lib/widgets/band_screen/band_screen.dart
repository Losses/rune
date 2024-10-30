import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../config/animation.dart';
import '../../widgets/no_items.dart';
import '../../widgets/infinite_list_loading.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../providers/responsive_providers.dart';

import '../start_screen/utils/group.dart';
import '../start_screen/utils/internal_collection.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';

class BandScreen extends StatefulWidget {
  final Future<List<Group<InternalCollection>>> Function() fetchSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(int) fetchPage;
  final Widget Function(BuildContext, InternalCollection, VoidCallback)
      itemBuilder;
  final bool userGenerated;

  const BandScreen({
    super.key,
    required this.fetchSummary,
    required this.fetchPage,
    required this.itemBuilder,
    required this.userGenerated,
  });

  @override
  BandScreenState createState() => BandScreenState();
}

class BandScreenState extends State<BandScreen> {
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
            final List<InternalCollection> flattenItems =
                items.expand((x) => x.items).toList();

            return DeviceTypeBuilder(
              deviceType: const [
                DeviceType.band,
                DeviceType.belt,
                DeviceType.dock,
                DeviceType.tv
              ],
              builder: (context, deviceType) {
                if (deviceType == DeviceType.band) {
                  return SmoothHorizontalScroll(
                    builder: (context, controller) {
                      return buildList(deviceType, flattenItems, controller);
                    },
                  );
                } else {
                  return buildList(deviceType, flattenItems, null);
                }
              },
            );
          }
        },
      ),
    );
  }

  InfiniteList buildList(
    DeviceType deviceType,
    List<InternalCollection> flattenItems,
    ScrollController? scrollController,
  ) {
    return InfiniteList(
      scrollController: scrollController,
      scrollDirection:
          deviceType == DeviceType.band || deviceType == DeviceType.belt
              ? Axis.horizontal
              : Axis.vertical,
      itemCount: flattenItems.length,
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
      onFetchData: _fetchData,
      hasReachedMax: isLastPage,
      itemBuilder: (context, index) {
        final item = flattenItems[index];
        return ManagedTurntileScreenItem(
          groupId: 0,
          row: index,
          column: 1,
          child: AspectRatio(
            aspectRatio: 1,
            child: Padding(
              padding: const EdgeInsets.symmetric(
                horizontal: 2,
                vertical: 1,
              ),
              child: widget.itemBuilder(context, item, _reloadData),
            ),
          ),
        );
      },
    );
  }
}
