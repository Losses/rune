import 'dart:async';

import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/api/fetch_library_summary.dart';
import '../../config/animation.dart';
import '../../widgets/start_screen/constants/default_gap_size.dart';
import '../../widgets/start_screen/link_tile.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/start_screen/utils/group.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../screens/collection/collection_item.dart';

import './constants/first_column.dart';

class LargeScreenLibraryHomeListView extends StatefulWidget {
  final String libraryPath;
  final StartScreenLayoutManager layoutManager;

  const LargeScreenLibraryHomeListView(
      {super.key, required this.libraryPath, required this.layoutManager});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<LargeScreenLibraryHomeListView> {
  Future<List<Group<dynamic>>>? summary;

  @override
  void initState() {
    setState(() {
      summary = fetchSummary();
    });

    super.initState();
  }

  @override
  dispose() {
    super.dispose();
  }

  Future<List<Group<InternalCollection>>> fetchSummary() async {
    final librarySummary = await fetchLibrarySummary();

    final groups = [
      Group<InternalCollection>(
        groupTitle: "Artists",
        items: librarySummary.artists
            .map(InternalCollection.fromRawCollection)
            .toList(),
      ),
      Group<InternalCollection>(
        groupTitle: "Albums",
        items: librarySummary.albums
            .map(InternalCollection.fromRawCollection)
            .toList(),
      ),
    ];

    Timer(
      Duration(milliseconds: gridAnimationDelay),
      () => widget.layoutManager.playAnimations(),
    );

    return groups;
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<Group<dynamic>>>(
      future: summary,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return Container();
        } else if (snapshot.hasError) {
          return Center(child: Text('Error: ${snapshot.error}'));
        } else if (!snapshot.hasData || snapshot.data!.isEmpty) {
          return const Center(child: Text('No data available'));
        } else {
          return LayoutBuilder(
            builder: (context, constraints) {
              return Container(
                alignment: Alignment.centerLeft,
                child: SmoothHorizontalScroll(
                  builder: (context, scrollController) => SingleChildScrollView(
                    scrollDirection: Axis.horizontal,
                    controller: scrollController,
                    child: Row(
                      mainAxisAlignment: MainAxisAlignment.start,
                      children: [
                        StartGroup<(String, String, IconData, bool)>(
                          groupIndex: 0,
                          groupTitle: 'Start',
                          items: smallScreenFirstColumn
                              .where((x) => !x.$4)
                              .toList(),
                          constraints: constraints,
                          groupLayoutVariation:
                              StartGroupGroupLayoutVariation.stacked,
                          gridLayoutVariation:
                              StartGroupGridLayoutVariation.initial,
                          gapSize: defaultGapSize,
                          onTitleTap: () {},
                          itemBuilder: (context, item) {
                            return LinkTile(
                              title: item.$1,
                              path: item.$2,
                              icon: item.$3,
                            );
                          },
                        ),
                        ...snapshot.data!.map(
                          (item) {
                            if (item is Group<InternalCollection>) {
                              return StartGroup<InternalCollection>(
                                groupIndex: item.groupTitle.hashCode,
                                groupTitle: item.groupTitle,
                                items: item.items,
                                constraints: constraints,
                                groupLayoutVariation:
                                    StartGroupGroupLayoutVariation.stacked,
                                gridLayoutVariation:
                                    StartGroupGridLayoutVariation.square,
                                gapSize: defaultGapSize,
                                onTitleTap: () => {
                                  context
                                      .push('/${item.groupTitle.toLowerCase()}')
                                },
                                itemBuilder: (context, item) {
                                  return CollectionItem(
                                    collectionType: item.collectionType,
                                    collection: item,
                                    refreshList: () {},
                                  );
                                },
                              );
                            } else {
                              return Container();
                            }
                          },
                        )
                      ],
                    ),
                  ),
                ),
              );
            },
          );
        }
      },
    );
  }
}
