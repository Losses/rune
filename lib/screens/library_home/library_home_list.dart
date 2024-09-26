import 'dart:async';

import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/api/fetch_library_summary.dart';
import '../../config/animation.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/tile/cover_art_manager.dart';
import '../../widgets/start_screen/start_group.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../messages/collection.pb.dart';

import '../collection/collection_list.dart';

class LibraryHomeListView extends StatefulWidget {
  final String libraryPath;
  final StartScreenLayoutManager layoutManager;

  const LibraryHomeListView(
      {super.key, required this.libraryPath, required this.layoutManager});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<LibraryHomeListView> {
  Future<List<Group<dynamic>>>? summary;
  final coverArtManager = CoverArtManager();

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

    coverArtManager.dispose();
  }

  Future<List<Group<InternalCollection>>> fetchSummary() async {
    final librarySummary = await fetchLibrarySummary();

    final groups = [
      Group<InternalCollection>(
        groupTitle: "Albums",
        items: librarySummary.albums
            .map(InternalCollection.fromRawCollection)
            .toList(),
      ),
      Group<InternalCollection>(
        groupTitle: "Artists",
        items: librarySummary.artists
            .map(InternalCollection.fromRawCollection)
            .toList(),
      )
    ];

    for (final group in groups) {
      for (final collection in group.items) {
        await coverArtManager.queryCoverArts(collection.queries);
      }
    }

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
          return Container(
            alignment: Alignment.centerLeft,
            child: SmoothHorizontalScroll(
              builder: (context, scrollController) => SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                controller: scrollController,
                child: Row(
                  mainAxisAlignment: MainAxisAlignment.start,
                  children: snapshot.data!.map((item) {
                    if (item is Group<InternalCollection>) {
                      return StartGroup<InternalCollection>(
                        groupIndex: 0,
                        groupTitle: item.groupTitle,
                        items: item.items,
                        groupLayoutVariation:
                            StartGroupGroupLayoutVariation.stacked,
                        gridLayoutVariation:
                            StartGroupGridLayoutVariation.square,
                        gapSize: 12,
                        onTitleTap: () => {context.push('/albums')},
                        itemBuilder: (context, item) {
                          return CollectionItem(
                            collectionType: CollectionType.Album,
                            collection: item,
                            coverArtIds:
                                coverArtManager.getResult(item.queries),
                          );
                        },
                      );
                    } else {
                      return Container();
                    }
                  }).toList(),
                ),
              ),
            ),
          );
        }
      },
    );
  }
}
