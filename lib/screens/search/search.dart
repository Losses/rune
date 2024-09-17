import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:get_storage/get_storage.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/context_menu/track_item_context_menu.dart';
import '../../widgets/cover_art.dart';
import '../../widgets/slide_fade_transition.dart';
import '../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../messages/album.pb.dart';
import '../../messages/artist.pb.dart';
import '../../messages/search.pb.dart';
import '../../messages/playback.pb.dart';
import '../../messages/media_file.pb.dart';
import '../../messages/playlist.pbserver.dart';

import './widgets/search_card.dart';

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});

  @override
  State<SearchPage> createState() => _SearchPageState();
}

const searchCategories = [
  'Tracks',
  'Artists',
  'Albums',
  'Playlists',
];
const searchIcons = [
  Symbols.music_note,
  Symbols.face,
  Symbols.album,
  Symbols.queue_music,
];

class _SearchPageState extends State<SearchPage> {
  final searchKey = GlobalKey(debugLabel: 'Search Bar Key');
  final searchFocusNode = FocusNode();
  final searchController = TextEditingController();

  String selectedItem = 'Tracks';
  Timer? _debounce;
  Timer? _saveDebounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

  final box = GetStorage();
  List<String> suggestions = [];

  List<MediaFile> tracks = [];
  List<Artist> artists = [];
  List<Album> albums = [];
  List<Playlist> playlists = [];

  String _lastSearched = '';

  final _layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();
    searchFocusNode.requestFocus();

    // Load suggestions from storage
    final storedSuggestions = box.read<List<dynamic>>('search_suggestions');
    if (storedSuggestions != null) {
      suggestions = List<String>.from(storedSuggestions);
    }

    searchController.addListener(() {
      _registerSearchTask();
      if (_saveDebounce?.isActive ?? false) _saveDebounce!.cancel();
      _saveDebounce = Timer(const Duration(seconds: 2), () {
        _saveQuery(searchController.text);
      });
    });
  }

  @override
  void dispose() {
    searchController.dispose();
    searchFocusNode.dispose();
    _debounce?.cancel();
    _saveDebounce?.cancel();
    _layoutManager.dispose();
    super.dispose();
  }

  void _registerSearchTask() {
    final task = searchController.text;

    if (_lastSearched == task) return;

    _lastSearched = task;
    if (_debounce?.isActive ?? false) _debounce!.cancel();
    _debounce = Timer(const Duration(milliseconds: 300), () {
      if (!_isRequestInProgress) {
        _performSearch(searchController.text);
      }
    });
  }

  Future<void> _performSearch(String query) async {
    if (_isRequestInProgress) return;
    setState(() {
      _isRequestInProgress = true;
    });

    try {
      _layoutManager.resetAnimations();

      final response = await searchFor(query);
      setState(() {
        _searchResults = response;
      });

      if (response.tracks.isNotEmpty) {
        tracks = await fetchMediaFileByIds(response.tracks);
      }
      if (response.artists.isNotEmpty) {
        artists = await fetchArtistsByIds(response.artists);
      }
      if (response.albums.isNotEmpty) {
        albums = await fetchAlbumsByIds(response.albums);
      }
      if (response.playlists.isNotEmpty) {
        playlists = await fetchPlaylistsByIds(response.playlists);
      }

      WidgetsBinding.instance.addPostFrameCallback((_) {
        _layoutManager.playAnimations();
      });
    } catch (e) {
      // Handle error
    } finally {
      setState(() {
        _isRequestInProgress = false;
      });
    }
  }

  void _saveQuery(String query) {
    if (_searchResults != null &&
        query.isNotEmpty &&
        !suggestions.contains(query)) {
      suggestions.add(query);
      if (suggestions.length > 64) {
        suggestions.removeAt(0); // Ensure we only keep the latest 64 queries
      }
      box.write('search_suggestions', suggestions);
    }
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final typography = theme.typography;

    final search = AutoSuggestBox<String>(
      key: searchKey,
      focusNode: searchFocusNode,
      controller: searchController,
      unfocusedColor: Colors.transparent,
      items: suggestions.map((suggestion) {
        return AutoSuggestBoxItem<String>(
          value: suggestion,
          label: suggestion,
          onSelected: () {
            searchController.text = suggestion;
            searchFocusNode.unfocus();
            _registerSearchTask();
          },
        );
      }).toList(),
      trailingIcon: IgnorePointer(
        child: IconButton(
          onPressed: () {},
          icon: const Icon(Symbols.search, size: 16,),
        ),
      ),
    );

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: _layoutManager,
        child: Row(children: [
          Expanded(
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const SizedBox(height: 12),
                  Text(selectedItem, style: typography.title),
                  const SizedBox(height: 24),
                  Expanded(
                      child: LayoutBuilder(builder: (context, constraints) {
                    const double gapSize = 8;
                    const double cellSize = 200;

                    const ratio = 4 / 1;

                    final int rows =
                        (constraints.maxWidth / (cellSize + gapSize)).floor();

                    return GridView(
                        key: Key(selectedItem),
                        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                          crossAxisCount: rows,
                          mainAxisSpacing: gapSize,
                          crossAxisSpacing: gapSize,
                          childAspectRatio: ratio,
                        ),
                        children: [
                          if (selectedItem == "Artists")
                            ...artists
                                .map((a) => ArtistItem(index: 0, item: a)),
                          if (selectedItem == "Albums")
                            ...albums.map((a) => AlbumItem(index: 0, item: a)),
                          if (selectedItem == "Playlists")
                            ...playlists
                                .map((a) => PlaylistItem(index: 0, item: a)),
                          if (selectedItem == "Tracks")
                            ...tracks.map((a) => TrackItem(index: 0, item: a)),
                        ].asMap().entries.map((x) {
                          final index = x.key;
                          final int row = index % rows;
                          final int column = index ~/ rows;

                          return ManagedStartScreenItem(
                              key: Key('$selectedItem-$row:$column'),
                              prefix: selectedItem,
                              groupId: 0,
                              row: row,
                              column: column,
                              width: cellSize / ratio,
                              height: cellSize,
                              child: x.value);
                        }).toList());
                  })),
                ],
              ),
            ),
          ),
          SizedBox(
            width: 320,
            child: SlideFadeTransition(
                child: Container(
              color: theme.cardColor,
              child: Padding(
                padding: const EdgeInsets.all(12),
                child: Column(
                  mainAxisAlignment: MainAxisAlignment.start,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Padding(
                      padding: const EdgeInsets.symmetric(
                          vertical: 13, horizontal: 16),
                      child: Text("Search", style: typography.bodyLarge),
                    ),
                    Padding(
                      padding: const EdgeInsets.symmetric(
                          vertical: 8, horizontal: 16),
                      child: SizedBox(
                        height: 36,
                        child: Row(
                          mainAxisSize: MainAxisSize.max,
                          children: [
                            Flexible(fit: FlexFit.loose, child: search)
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(height: 12),
                    Expanded(
                      child: ListView.builder(
                        itemCount: searchCategories.length,
                        itemBuilder: (context, index) {
                          final item = searchCategories[index];
                          int itemCount = 0;
                          if (_searchResults != null) {
                            switch (item) {
                              case 'Artists':
                                itemCount = artists.length;
                                break;
                              case 'Albums':
                                itemCount = albums.length;
                                break;
                              case 'Playlists':
                                itemCount = playlists.length;
                                break;
                              case 'Tracks':
                                itemCount = tracks.length;
                                break;
                            }
                          }
                          return ListTile.selectable(
                            leading: Container(
                              width: 36,
                              height: 36,
                              decoration: BoxDecoration(
                                color: theme.accentColor,
                                borderRadius: BorderRadius.circular(2),
                              ),
                              child: Icon(searchIcons[index],
                                  color: theme.activeColor, size: 26),
                            ),
                            title: Row(
                              children: [
                                Expanded(
                                    child: Text(item, style: typography.body)),
                                if (itemCount > 0)
                                  Text(
                                    '$itemCount',
                                    style: typography.body!.copyWith(
                                      color: theme.inactiveColor.withAlpha(160),
                                    ),
                                  ),
                              ],
                            ),
                            selectionMode: ListTileSelectionMode.single,
                            selected: selectedItem == item,
                            onSelectionChange: (v) {
                              _layoutManager.resetAnimations();

                              setState(() {
                                selectedItem = item;
                              });

                              WidgetsBinding.instance.addPostFrameCallback((_) {
                                _layoutManager.playAnimations();
                              });
                            },
                          );
                        },
                      ),
                    ),
                  ],
                ),
              ),
            )),
          ),
        ]));
  }
}

class TrackItem extends SearchCard {
  final MediaFile item;

  TrackItem({
    super.key,
    required super.index,
    required this.item,
  });

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.title;

  @override
  Widget buildLeadingWidget(double size) {
    return CoverArt(
      fileId: item.id,
      size: size,
    );
  }

  @override
  void onPressed(BuildContext context) {
    PlayFileRequest(fileId: item.id).sendSignalToRust();
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openTrackItemContextMenu(
        position, context, contextAttachKey, contextController, item.id);
  }
}

class ArtistItem extends CollectionSearchCard<Artist> {
  ArtistItem({
    super.key,
    required super.index,
    required super.item,
  }) : super(routePrefix: 'artists', emptyTileType: BoringAvatarType.marble);

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.name;

  @override
  List<int> getCoverIds() => item.coverIds;
}

class AlbumItem extends CollectionSearchCard<Album> {
  AlbumItem({
    super.key,
    required super.index,
    required super.item,
  }) : super(routePrefix: 'albums', emptyTileType: BoringAvatarType.bauhaus);

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.name;

  @override
  List<int> getCoverIds() => item.coverIds;
}

class PlaylistItem extends CollectionSearchCard<Playlist> {
  PlaylistItem({
    super.key,
    required super.index,
    required super.item,
  }) : super(
            routePrefix: 'playlists', emptyTileType: BoringAvatarType.bauhaus);

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.name;

  @override
  List<int> getCoverIds() => item.coverIds;
}

Future<SearchForResponse> searchFor(String query) async {
  final searchRequest = SearchForRequest(queryStr: query, n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  return (await SearchForResponse.rustSignalStream.first).message;
}

Future<List<MediaFile>> fetchMediaFileByIds(List<int> ids) async {
  final request = FetchMediaFileByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchMediaFileByIdsResponse.rustSignalStream.first)
      .message
      .result;
}

Future<List<Album>> fetchAlbumsByIds(List<int> ids) async {
  final request = FetchAlbumsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchAlbumsByIdsResponse.rustSignalStream.first).message.result;
}

Future<List<Artist>> fetchArtistsByIds(List<int> ids) async {
  final request = FetchArtistsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchArtistsByIdsResponse.rustSignalStream.first)
      .message
      .result;
}

Future<List<Playlist>> fetchPlaylistsByIds(List<int> ids) async {
  final request = FetchPlaylistsByIdsRequest(ids: ids);
  request.sendSignalToRust(); // GENERATED

  return (await FetchPlaylistsByIdsResponse.rustSignalStream.first)
      .message
      .result;
}
