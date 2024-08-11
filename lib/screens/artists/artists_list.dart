import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../widgets/flip_grid.dart';
import '../../widgets/start_screen.dart';
import '../../../messages/artist.pb.dart';

class ArtistsListView extends StatefulWidget {
  const ArtistsListView({super.key});

  @override
  ArtistsListViewState createState() => ArtistsListViewState();
}

class ArtistsListViewState extends State<ArtistsListView> {
  static const _pageSize = 3;

  final PagingController<int, ArtistsGroup> _pagingController =
      PagingController(firstPageKey: 0);

  late Future<List<ArtistsGroupSummary>> summary;

  Future<List<Group<Artist>>> fetchArtistSummary() async {
    final fetchArtistsGroupSummary = FetchArtistsGroupSummaryRequest();
    fetchArtistsGroupSummary.sendSignalToRust(); // GENERATED

    // Listen for the response from Rust
    final rustSignal = await ArtistGroupSummaryResponse.rustSignalStream.first;
    final artistGroupList = rustSignal.message;

    return artistGroupList.artistsGroups.map((summary) {
      return Group<Artist>(
        groupTitle: summary.groupTitle,
        items: [], // Initially empty, will be filled in fetchPage
      );
    }).toList();
  }

  Future<void> fetchArtistPage(
    PagingController<int, Group<Artist>> controller,
    int cursor,
  ) async {
    try {
      final summaries = await fetchArtistSummary();

      // Calculate the start and end index for the current page
      final startIndex = cursor * _pageSize;
      final endIndex = (cursor + 1) * _pageSize;

      // Check if we have more data to load
      if (startIndex >= summaries.length) {
        controller.appendLastPage([]);
        return;
      }

      // Get the current page's group titles
      final currentPageSummaries = summaries.sublist(
        startIndex,
        endIndex > summaries.length ? summaries.length : endIndex,
      );

      // Extract group titles for the current page
      final groupTitles =
          currentPageSummaries.map((summary) => summary.groupTitle).toList();

      // Create request for fetching artist groups
      final fetchArtistsGroupsRequest = FetchArtistsGroupsRequest()
        ..groupTitles.addAll(groupTitles);
      fetchArtistsGroupsRequest.sendSignalToRust(); // GENERATED

      // Listen for the response from Rust
      final rustSignal = await ArtistsGroups.rustSignalStream.first;
      final artistsGroups = rustSignal.message.groups;

      final groups = artistsGroups.map((group) {
        return Group<Artist>(
          groupTitle: group.groupTitle,
          items: group.artists,
        );
      }).toList();

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

  @override
  Widget build(BuildContext context) {
    return StartScreen<Artist>(
      fetchSummary: fetchArtistSummary,
      fetchPage: fetchArtistPage,
      itemBuilder: (context, artist) => ArtistItem(artist: artist),
    );
  }

  @override
  void dispose() {
    _pagingController.dispose();
    super.dispose();
  }
}

class ArtistItem extends StatelessWidget {
  final Artist artist;

  const ArtistItem({
    super.key,
    required this.artist,
  });

  @override
  Widget build(BuildContext context) {
    return Button(
      style:
          const ButtonStyle(padding: WidgetStatePropertyAll(EdgeInsets.all(0))),
      onPressed: () => {},
      child: ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox(
          width: double.infinity,
          height: double.infinity,
          child: Stack(
            alignment: Alignment.bottomLeft,
            children: [
              FlipCoverGrid(numbers: artist.coverIds, id: artist.name),
              IgnorePointer(
                ignoring: true,
                child: Container(
                  decoration: BoxDecoration(
                    gradient: LinearGradient(
                      begin: const Alignment(0.0, -1.0),
                      end: const Alignment(0.0, 1.0),
                      colors: [
                        Colors.black.withAlpha(0),
                        Colors.black.withAlpha(160),
                      ],
                    ),
                  ),
                  height: 80,
                ),
              ),
              Padding(
                padding: const EdgeInsets.all(6),
                child: Text(
                  artist.name,
                  textAlign: TextAlign.start,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
