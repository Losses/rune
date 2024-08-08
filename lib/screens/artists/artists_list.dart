import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:infinite_scroll_pagination/infinite_scroll_pagination.dart';

import '../../../utils/platform.dart';
import '../../../messages/artist.pb.dart';

class ArtistListView extends StatefulWidget {
  const ArtistListView({super.key});

  @override
  ArtistListViewState createState() => ArtistListViewState();
}

class ArtistListViewState extends State<ArtistListView> {
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
          return PagedListView<int, ArtistsGroup>(
            pagingController: _pagingController,
            builderDelegate: PagedChildBuilderDelegate<ArtistsGroup>(
              itemBuilder: (context, item, index) => ArtistListItem(
                index: index,
                groupTitle: item.groupTitle,
                items: item.artists,
              ),
            ),
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

class ArtistListItem extends StatelessWidget {
  final List<Artist> items;
  final String groupTitle;
  final int index;

  final contextController = FlyoutController();
  final contextAttachKey = GlobalKey();

  ArtistListItem({
    super.key,
    required this.index,
    required this.groupTitle,
    required this.items,
  });

  openContextMenu(Offset localPosition, BuildContext context) {
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    contextController.showFlyout(
      barrierColor: Colors.black.withOpacity(0.1),
      position: position,
      builder: (context) {
        var items = [
          MenuFlyoutItem(
            leading: const Icon(Symbols.rocket),
            text: const Text('Roaming'),
            onPressed: () => {
              // Do something here
            },
          ),
        ];

        return MenuFlyout(
          items: items,
        );
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
        key: contextAttachKey,
        controller: contextController,
        child: ListTile.selectable(
            title: Column(
              children: [
                Text(groupTitle),
                Column(
                  children: items
                      .map((x) => GestureDetector(
                          onSecondaryTapUp: isDesktop
                              ? (d) {
                                  openContextMenu(d.localPosition, context);
                                }
                              : null,
                          onLongPressEnd: isDesktop
                              ? null
                              : (d) {
                                  openContextMenu(d.localPosition, context);
                                },
                          child: Text(x.name)))
                      .toList(),
                )
              ],
            ),
            onSelectionChange: (v) => {}));
  }
}
