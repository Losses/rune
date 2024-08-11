import 'package:fluent_ui/fluent_ui.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';
import 'package:player/widgets/flip_grid.dart';

import '../../../messages/artist.pb.dart';
import '../../../widgets/start_grid.dart';
import '../../../widgets/smooth_horizontal_scroll.dart';

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

  @override
  void initState() {
    super.initState();
    summary = fetchSummary();
    _pagingController.addPageRequestListener((cursor) {
      _fetchPage(cursor);
    });
  }

  Future<List<ArtistsGroupSummary>> fetchSummary() async {
    final fetchArtistsGroupSummary = FetchArtistsGroupSummaryRequest();
    fetchArtistsGroupSummary.sendSignalToRust(); // GENERATED

    // Listen for the response from Rust
    final rustSignal = await ArtistGroupSummaryResponse.rustSignalStream.first;
    final artistGroupList = rustSignal.message;
    return artistGroupList.artistsGroups;
  }

  Future<void> _fetchPage(int cursor) async {
    try {
      // Ensure summary is loaded
      final summaries = await summary;

      // Calculate the start and end index for the current page
      final startIndex = cursor * _pageSize;
      final endIndex = (cursor + 1) * _pageSize;

      // Check if we have more data to load
      if (startIndex >= summaries.length) {
        _pagingController.appendLastPage([]);
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

      // Check if we have reached the last page
      final isLastPage = endIndex >= summaries.length;
      if (isLastPage) {
        _pagingController.appendLastPage(artistsGroups);
      } else {
        _pagingController.appendPage(artistsGroups, cursor + 1);
      }
    } catch (error) {
      _pagingController.error = error;
    }
  }

  @override
  Widget build(BuildContext context) {
    return FutureBuilder<List<ArtistsGroupSummary>>(
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
                    PagedListView<int, ArtistsGroup>(
                      pagingController: _pagingController,
                      scrollDirection: Axis.horizontal,
                      scrollController: scrollController,
                      builderDelegate: PagedChildBuilderDelegate<ArtistsGroup>(
                        itemBuilder: (context, item, index) => ArtistGroup(
                          index: index,
                          groupTitle: item.groupTitle,
                          items: item.artists,
                        ),
                      ),
                    )),
          );
        }
      },
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

class ArtistGroup extends StatelessWidget {
  final List<Artist> items;
  final String groupTitle;
  final int index;

  const ArtistGroup({
    super.key,
    required this.index,
    required this.groupTitle,
    required this.items,
  });

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Container(
      padding: const EdgeInsets.only(left: 16, right: 16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 4),
            child: Text(groupTitle, style: theme.typography.bodyLarge),
          ),
          Expanded(
            child: StartGrid(
              cellSize: 120,
              gapSize: 4,
              children: items
                  .map((x) => ArtistItem(
                        artist: x,
                      ))
                  .toList(),
            ),
          ),
        ],
      ),
    );
  }
}
