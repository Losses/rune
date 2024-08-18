import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/album.pb.dart';
import '../../messages/artist.pb.dart';
import '../../messages/library_home.pb.dart';

import '../../screens/albums/albums_list.dart';
import '../../screens/artists/artists_list.dart';

import '../../widgets/start_screen.dart';
import '../../widgets/smooth_horizontal_scroll.dart';

class LibraryHomeListView extends StatefulWidget {
  const LibraryHomeListView({super.key});

  @override
  LibraryHomeListState createState() => LibraryHomeListState();
}

class LibraryHomeListState extends State<LibraryHomeListView> {
  late Future<List<Group<dynamic>>> summary;

  @override
  void initState() {
    super.initState();
    summary = fetchSummary();
  }

  Future<List<Group<dynamic>>> fetchSummary() async {
    final fetchLibrarySummary = FetchLibrarySummaryRequest();
    fetchLibrarySummary.sendSignalToRust(); // GENERATED

    final rustSignal = await LibrarySummaryResponse.rustSignalStream.first;
    final librarySummary = rustSignal.message;

    return [
      Group<Album>(groupTitle: "Albums", items: librarySummary.albums),
      Group<Artist>(groupTitle: "Artists", items: librarySummary.artists)
    ];
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<Group<dynamic>>>(
      future: summary,
      builder: (context, snapshot) {
        if (snapshot.connectionState == ConnectionState.waiting) {
          return const Text("Loading...");
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
                            if (item is Group<Album>) {
                              return StartGroup<Album>(
                                index: 0,
                                groupTitle: item.groupTitle,
                                items: item.items,
                                gapSize: 8,
                                variation: StartGroupVariation.square,
                                itemBuilder:
                                    (BuildContext context, Album item) =>
                                        AlbumItem(album: item),
                              );
                            } else if (item is Group<Artist>) {
                              return StartGroup<Artist>(
                                index: 1,
                                groupTitle: item.groupTitle,
                                items: item.items,
                                gapSize: 8,
                                variation: StartGroupVariation.square,
                                itemBuilder:
                                    (BuildContext context, Artist item) =>
                                        ArtistItem(artist: item),
                              );
                            } else {
                              return Container();
                            }
                          }).toList(),
                        ),
                      )));
        }
      },
    );
  }
}
