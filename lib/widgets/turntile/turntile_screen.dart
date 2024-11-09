import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:very_good_infinite_list/very_good_infinite_list.dart';

import '../../config/animation.dart';
import '../../screens/collection/utils/is_user_generated.dart';
import '../../screens/collection/utils/collection_item_builder.dart';
import '../../widgets/no_items.dart';
import '../../screens/collection/utils/collection_data_provider.dart';

import '../infinite_list_loading.dart';
import '../start_screen/utils/group.dart';
import '../start_screen/utils/internal_collection.dart';
import '../start_screen/providers/start_screen_layout_manager.dart';
import '../navigation_bar/page_content_frame.dart';

import 'turntile_group.dart';

class TurntileScreen extends StatefulWidget {
  const TurntileScreen({super.key});

  @override
  TurntileScreenState createState() => TurntileScreenState();
}

class TurntileScreenState extends State<TurntileScreen> {
  final layoutManager = StartScreenLayoutManager();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();

    Provider.of<CollectionDataProvider>(context, listen: false).summary.then(
      (x) {
        Timer(
          Duration(milliseconds: gridAnimationDelay),
          () => layoutManager.playAnimations(),
        );
      },
    );
  }

  @override
  void dispose() {
    super.dispose();
    layoutManager.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final data = Provider.of<CollectionDataProvider>(context);
    final isUserGenerated = userGenerated(data.collectionType);

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: FutureBuilder<List<Group<InternalCollection>>>(
        future: data.summary,
        builder: (context, snapshot) {
          if (snapshot.connectionState == ConnectionState.waiting) {
            return Container();
          } else if (snapshot.hasError) {
            return Center(child: Text('Error: ${snapshot.error}'));
          } else {
            return InfiniteList(
              itemCount: data.items.length,
              loadingBuilder: (context) => const InfiniteListLoading(),
              centerLoading: true,
              centerEmpty: true,
              isLoading: data.isLoading,
              padding: getScrollContainerPadding(context),
              emptyBuilder: (context) => Center(
                child: data.initialized
                    ? NoItems(
                        title: "No collection found",
                        hasRecommendation: false,
                        reloadData: data.reloadData,
                        userGenerated: isUserGenerated,
                      )
                    : Container(),
              ),
              onFetchData: data.fetchData,
              hasReachedMax: data.isLastPage,
              itemBuilder: (context, index) {
                final item = data.items[index];
                return TurntileGroup<InternalCollection>(
                  key: Key(item.groupTitle),
                  groupIndex: index,
                  groupTitle: item.groupTitle,
                  gridLayoutVariation: TurntileGroupGridLayoutVariation.tile,
                  items: item.items,
                  itemBuilder: (context, item) =>
                      collectionItemBuilder(context, item),
                );
              },
            );
          }
        },
      ),
    );
  }
}
