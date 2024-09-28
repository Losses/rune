import 'dart:async';

import 'package:player/widgets/track_list/track_list.dart';
import 'package:provider/provider.dart';
import 'package:go_router/go_router.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:get_storage/get_storage.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:flutter_boring_avatars/flutter_boring_avatars.dart';

import '../../utils/query_list.dart';
import '../../utils/router_extra.dart';
import '../../utils/api/search_for.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/api/fetch_media_file_by_ids.dart';
import '../../utils/api/operate_playback_with_mix_query.dart';
import '../../utils/context_menu/track_item_context_menu.dart';
import '../../utils/context_menu/collection_item_context_menu.dart';
import '../../widgets/tile/flip_grid.dart';
import '../../widgets/tile/cover_art.dart';
import '../../widgets/slide_fade_transition.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../screens/collection/collection_list.dart';
import '../../messages/search.pb.dart';
import '../../messages/collection.pb.dart';

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

  List<InternalMediaFile> tracks = [];
  List<InternalCollection> artists = [];
  List<InternalCollection> albums = [];
  List<InternalCollection> playlists = [];

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
        tracks = await fetchMediaFileByIds(response.tracks, true);
      }
      if (response.artists.isNotEmpty) {
        artists = (await fetchCollectionByIds(
          CollectionType.Artist,
          response.artists,
        ))
            .map(InternalCollection.fromRawCollection)
            .toList();
      }
      if (response.albums.isNotEmpty) {
        albums = (await fetchCollectionByIds(
          CollectionType.Album,
          response.albums,
        ))
            .map(InternalCollection.fromRawCollection)
            .toList();
      }
      if (response.playlists.isNotEmpty) {
        playlists = (await fetchCollectionByIds(
          CollectionType.Playlist,
          response.playlists,
        ))
            .map(InternalCollection.fromRawCollection)
            .toList();
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
          icon: const Icon(
            Symbols.search,
            size: 16,
          ),
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

                    final trackIds = tracks.map((x) => x.id).toList();

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
                            ...artists.map(
                              (a) => CollectionItem(
                                item: a,
                                collectionType: CollectionType.Artist,
                              ),
                            ),
                          if (selectedItem == "Albums")
                            ...albums.map(
                              (a) => CollectionItem(
                                item: a,
                                collectionType: CollectionType.Album,
                              ),
                            ),
                          if (selectedItem == "Playlists")
                            ...playlists.map(
                              (a) => CollectionItem(
                                item: a,
                                collectionType: CollectionType.Playlist,
                              ),
                            ),
                          if (selectedItem == "Tracks")
                            ...tracks.map((a) => TrackSearchItem(
                                  index: 0,
                                  item: a,
                                  fallbackFileIds: trackIds,
                                )),
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
                            child: x.value,
                          );
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
                        vertical: 13,
                        horizontal: 16,
                      ),
                      child: Text("Search", style: typography.bodyLarge),
                    ),
                    Padding(
                      padding: const EdgeInsets.symmetric(
                        vertical: 8,
                        horizontal: 16,
                      ),
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
                              child: Icon(
                                searchIcons[index],
                                color: theme.activeColor,
                                size: 26,
                              ),
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

class TrackSearchItem extends SearchCard {
  final InternalMediaFile item;
  final List<int> fallbackFileIds;

  TrackSearchItem({
    super.key,
    required super.index,
    required this.item,
    required this.fallbackFileIds,
  });

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.title;

  @override
  Widget buildLeadingWidget(double size) {
    return CoverArt(
      path: item.coverArtPath,
      size: size,
    );
  }

  @override
  void onPressed(BuildContext context) {
    operatePlaybackWithMixQuery(
      queries: const QueryList([]),
      playbackMode: 99,
      hintPosition: 0,
      initialPlaybackId: item.id,
      replacePlaylist: true,
      instantlyPlay: true,
      fallbackFileIds: fallbackFileIds,
    );
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openTrackItemContextMenu(
        position, context, contextAttachKey, contextController, item.id);
  }
}

class CollectionItem extends SearchCard {
  final InternalCollection item;
  final CollectionType collectionType;
  final BoringAvatarType emptyTileType = BoringAvatarType.marble;

  CollectionItem({
    super.key,
    super.index = 0,
    required this.item,
    required this.collectionType,
  });

  @override
  void onPressed(BuildContext context) {
    context.replace('/${routerName[collectionType]}/${getItemId()}',
        extra: QueryTracksExtra(getItemTitle()));
  }

  @override
  void onContextMenu(BuildContext context, Offset position) {
    openCollectionItemContextMenu(
      position,
      context,
      contextAttachKey,
      contextController,
      collectionType,
      getItemId(),
    );
  }

  @override
  int getItemId() => item.id;

  @override
  String getItemTitle() => item.name;

  @override
  Widget buildLeadingWidget(double size) {
    return SizedBox(
      width: size,
      height: size,
      child: FlipCoverGrid(
        id: getItemTitle(),
        paths: item.coverArtMap.values.toList(),
        emptyTileType: BoringAvatarType.bauhaus,
      ),
    );
  }
}
