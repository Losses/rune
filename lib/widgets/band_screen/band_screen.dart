import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../config/animation.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../screens/collection/utils/collection_data_provider.dart';
import '../../widgets/no_items.dart';
import '../../widgets/infinite_list_loading.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/turntile/managed_turntile_screen_item.dart';
import '../../providers/responsive_providers.dart';
import '../../generated/l10n.dart';

import '../start_screen/utils/group.dart';
import '../start_screen/utils/internal_collection.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';
import '../navigation_bar/page_content_frame.dart';

class BandScreen extends StatefulWidget {
  const BandScreen({super.key});

  @override
  BandScreenState createState() => BandScreenState();
}

class BandScreenState extends State<BandScreen> {
  final layoutManager = StartScreenLayoutManager();

  @override
  void dispose() {
    super.dispose();
    layoutManager.dispose();
  }

  _fetchData() async {
    final data = Provider.of<CollectionDataProvider>(context, listen: false);
    await data.fetchData();

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => layoutManager.playAnimations(),
    );
  }

  @override
  Widget build(BuildContext context) {
    final data = Provider.of<CollectionDataProvider>(context);

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return const Center(
              child: ProgressRing(),
            );
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else {
            final List<InternalCollection> flattenItems =
                data.items.expand((x) => x.items).toList();

            return DeviceTypeBuilder(
              deviceType: const [
                DeviceType.band,
                DeviceType.belt,
                DeviceType.station,
                DeviceType.dock,
                DeviceType.tv
              ],
              builder: (context, deviceType) {
                if (deviceType == DeviceType.band ||
                    deviceType == DeviceType.belt) {
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
    final data = Provider.of<CollectionDataProvider>(context);

    final isUserGenerated = userGenerated(data.collectionType);

    return InfiniteList(
      scrollDirection:
          deviceType == DeviceType.band || deviceType == DeviceType.belt
              ? Axis.horizontal
              : Axis.vertical,
      scrollController: scrollController,
      itemCount: flattenItems.length,
      loadingBuilder: (context) => const InfiniteListLoading(),
      centerLoading: true,
      centerEmpty: true,
      isLoading: data.isLoading,
      padding: getScrollContainerPadding(context),
      emptyBuilder: (context) => Center(
        child: data.initialized
            ? NoItems(
                title: S.of(context).noCollectionFound,
                hasRecommendation: false,
                reloadData: data.reloadData,
                userGenerated: isUserGenerated,
              )
            : Container(),
      ),
      onFetchData: _fetchData,
      hasReachedMax: data.isLastPage,
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
              child: collectionItemBuilder(context, item),
            ),
          ),
        );
      },
    );
  }
}
