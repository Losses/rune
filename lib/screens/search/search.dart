import 'dart:io';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/material_symbols_icons.dart';

import '../../utils/kept_clear.dart';
import '../../utils/l10n.dart';
import '../../utils/router/router_aware_flyout_controller.dart';
import '../../utils/api/search_for.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/api/fetch_media_file_by_ids.dart';
import '../../widgets/context_menu_wrapper.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/search/widgets/small_screen_search_track_list.dart';
import '../../screens/search/constants/search_categories.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';
import '../../providers/router_path.dart';
import '../../utils/rune_log.dart';

import 'utils/track_items_to_search_card.dart';
import 'utils/collection_items_to_search_card.dart';
import 'widgets/search_card.dart';
import 'widgets/search_suggest_box.dart';
import 'widgets/collection_search_item.dart';
import 'widgets/large_screen_search_sidebar.dart';
import 'widgets/band_screen_search_track_list.dart';
import 'widgets/large_screen_search_track_list.dart';
import 'widgets/medium_screen_search_track_list.dart';

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});

  @override
  State<SearchPage> createState() => _SearchPageState();
}

class _SearchPageState extends State<SearchPage> {
  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.dock,
        DeviceType.band,
        DeviceType.zune,
        DeviceType.tablet,
        DeviceType.tv
      ],
      builder: (context, deviceType) {
        return Actions(
          actions: const {},
          child: SearchPageImplementation(
            deviceType: deviceType,
          ),
        );
      },
    );
  }
}

class SearchPageImplementation extends StatefulWidget {
  final DeviceType deviceType;
  const SearchPageImplementation({super.key, required this.deviceType});

  @override
  State<SearchPageImplementation> createState() =>
      _SearchPageImplementationState();
}

class _SearchPageImplementationState extends State<SearchPageImplementation> {
  final searchController = TextEditingController();
  final GlobalKey<SearchSuggestBoxState> _searchBoxKey = GlobalKey();

  (CollectionType, String Function(BuildContext)) selectedItem =
      searchCategories[0];
  Timer? _debounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

  // A search that takes longer than this shows a progress ring.
  static const _ringDelay = Duration(milliseconds: 250);
  Timer? _ringTimer;
  bool _showRing = false;
  // A refresh asked for while a search was running, answered when it is done.
  bool _pendingRefresh = false;

  // Covers arrive after the results, and are read in again one at a time.
  StreamSubscription? _contentSubscription;
  bool _rereading = false;
  bool _rereadAgain = false;

  // Whether the router showed this page at the last change. Pages opened from
  // here are pushed on top of it, so it stays mounted while they are shown.
  bool _onSearchPage = false;

  final contextController = RouterAwareFlyoutController();
  final contextAttachKey = GlobalKey();

  Map<CollectionType, List<SearchCard>> items = {};

  String _lastSearched = '';

  final bandScreenLayoutManager = StartScreenLayoutManager();
  final largeScreenLayoutManager = StartScreenLayoutManager();
  final mediumScreenLayoutManager = StartScreenLayoutManager();
  final smallScreenLayoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();

    searchController.addListener(_registerSearchTask);
    _onSearchPage = $router.path == '/search';
    $router.addListener(_onRouteChanged);
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      // A covered page searches again when it is shown again.
      if (mounted && _onSearchPage) _rereadResults();
    });
  }

  @override
  void dispose() {
    super.dispose();
    $router.removeListener(_onRouteChanged);
    _contentSubscription?.cancel();
    contextController.dispose();
    _debounce?.cancel();
    _ringTimer?.cancel();
    searchController.dispose();
    bandScreenLayoutManager.dispose();
    largeScreenLayoutManager.dispose();
    mediumScreenLayoutManager.dispose();
    smallScreenLayoutManager.dispose();
  }

  void resetAnimations() {
    bandScreenLayoutManager.resetAnimations();
    largeScreenLayoutManager.resetAnimations();
    mediumScreenLayoutManager.resetAnimations();
    smallScreenLayoutManager.resetAnimations();
  }

  void playAnimations() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      bandScreenLayoutManager.playAnimations();
      largeScreenLayoutManager.playAnimations();
      mediumScreenLayoutManager.playAnimations();
      smallScreenLayoutManager.playAnimations();
    });
  }

  @override
  void didUpdateWidget(covariant SearchPageImplementation oldWidget) {
    super.didUpdateWidget(oldWidget);

    if (oldWidget.deviceType != widget.deviceType) {
      resetAnimations();
      playAnimations();
    }
  }

  void _registerSearchTask() {
    final task = searchController.text;

    if (_lastSearched == task) return;

    _lastSearched = task;
    if (_debounce?.isActive ?? false) _debounce!.cancel();

    // An empty box shows nothing; a search still running is dropped when it
    // finishes, since the text no longer matches.
    if (task.trim().isEmpty) {
      _clearResults();
      return;
    }

    _debounce = Timer(const Duration(milliseconds: 300), () {
      if (!_isRequestInProgress && searchController.text.trim().isNotEmpty) {
        _performSearch(searchController.text);
      }
    });
  }

  Future<List<CollectionSearchItem>> collectionResponseToSearchCard(
    List<int> ids,
    CollectionType type,
  ) async {
    return collectionItemsToSearchCard(
      (await fetchCollectionByIds(type, ids, bakeCoverArts: true))
          .map(InternalCollection.fromRawCollection)
          .toList(),
      type,
    );
  }

  Future<Map<CollectionType, List<SearchCard>>> _cardsFor(
    SearchForResponse response,
  ) async {
    final Map<CollectionType, List<SearchCard>> found = {};

    if (response.tracks.isNotEmpty) {
      found[CollectionType.track] = trackItemsToSearchCard(
        await fetchMediaFileByIds(response.tracks, true),
      );
    }
    if (response.artists.isNotEmpty) {
      found[CollectionType.artist] = await collectionResponseToSearchCard(
        response.artists,
        CollectionType.artist,
      );
    }
    if (response.albums.isNotEmpty) {
      found[CollectionType.album] = await collectionResponseToSearchCard(
        response.albums,
        CollectionType.album,
      );
    }
    if (response.playlists.isNotEmpty) {
      found[CollectionType.playlist] = await collectionResponseToSearchCard(
        response.playlists,
        CollectionType.playlist,
      );
    }
    if (response.genres.isNotEmpty) {
      found[CollectionType.genre] = await collectionResponseToSearchCard(
        response.genres,
        CollectionType.genre,
      );
    }
    return found;
  }

  bool _sameIds(SearchForResponse a, SearchForResponse? b) {
    return b != null &&
        listEquals(a.tracks, b.tracks) &&
        listEquals(a.artists, b.artists) &&
        listEquals(a.albums, b.albums) &&
        listEquals(a.playlists, b.playlists) &&
        listEquals(a.genres, b.genres);
  }

  /// Searches for [query]. A [quiet] search shows no ring, keeps what is shown
  /// until the answer arrives, and replaces it only when the ids differ.
  Future<void> _performSearch(
    String query, {
    bool refresh = false,
    bool quiet = false,
  }) async {
    if (_isRequestInProgress) return;
    _isRequestInProgress = true;

    if (!quiet) {
      _ringTimer?.cancel();
      _ringTimer = Timer(_ringDelay, () {
        if (mounted &&
            _isRequestInProgress &&
            searchController.text.trim().isNotEmpty) {
          setState(() => _showRing = true);
        }
      });
    }

    try {
      final response = await searchFor(query, refresh: refresh);
      if (quiet && _sameIds(response, _searchResults)) return;

      final found = await _cardsFor(response);

      // The text changed while this search ran; its results are out of date
      // and the new text is searched below.
      if (!mounted || searchController.text != query) return;

      setState(() {
        _searchResults = response;
        items = found;
        _showRing = false;
        if (!quiet) {
          resetAnimations();
          playAnimations();
        }
      });
    } catch (e) {
      // A failed request is reported by the hub; the page keeps searching.
      error$('Search for "$query" failed: $e');
      // Do not leave the results of an earlier text under this one.
      if (!quiet && mounted && searchController.text == query) _clearResults();
    } finally {
      _ringTimer?.cancel();
      _isRequestInProgress = false;
      if (mounted && _showRing) setState(() => _showRing = false);
      if (_pendingRefresh) {
        _pendingRefresh = false;
        _refresh();
      } else {
        _catchUp(query);
      }
    }
  }

  /// Searches the box's text again, past what the hub kept of an earlier
  /// search.
  void _refresh() {
    if (!mounted || searchController.text.trim().isEmpty) return;
    if (_isRequestInProgress) {
      _pendingRefresh = true;
      return;
    }
    _performSearch(searchController.text, refresh: true);
  }

  void _onRouteChanged() {
    final onSearch = $router.path == '/search';
    final returned = onSearch && !_onSearchPage;
    _onSearchPage = onSearch;
    if (!returned || !mounted) return;

    final text = searchController.text;
    if (text.trim().isEmpty || (_debounce?.isActive ?? false)) return;
    _performSearch(text, quiet: true);
  }

  /// Reads the shown results in again by their ids, which fills in the covers
  /// that arrived after them. One read runs at a time, and one more follows it
  /// when something changed meanwhile.
  Future<void> _rereadResults() async {
    if (_rereading) {
      _rereadAgain = true;
      return;
    }
    _rereading = true;
    try {
      do {
        _rereadAgain = false;
        final response = _searchResults;
        if (response == null) break;
        final found = await _cardsFor(response);
        // A newer search replaced these results; it read its own.
        if (!mounted || !identical(_searchResults, response)) break;
        setState(() => items = found);
      } while (_rereadAgain);
    } catch (e) {
      error$('Reading the search results again failed: $e');
    } finally {
      _rereading = false;
    }
  }

  void openSearchContextMenu(Offset localPosition) {
    if (!context.mounted) return;
    if (searchController.text.trim().isEmpty) return;
    final targetContext = contextAttachKey.currentContext;

    if (targetContext == null) return;
    final box = targetContext.findRenderObject() as RenderBox;
    final position = box.localToGlobal(
      localPosition,
      ancestor: Navigator.of(context).context.findRenderObject(),
    );

    contextController.showFlyout(
      position: position,
      builder: (context) {
        return MenuFlyout(
          items: [
            MenuFlyoutItem(
              leading: const Icon(Symbols.refresh),
              text: Text(S.of(context).refresh),
              onPressed: _refresh,
            ),
          ],
        );
      },
    );
  }

  /// The results area: the results, or a ring while a search is slow, with the
  /// context menu on it.
  Widget _resultsArea(Widget child) {
    return ContextMenuWrapper(
      contextAttachKey: contextAttachKey,
      contextController: contextController,
      onContextMenu: openSearchContextMenu,
      onMiddleClick: (_) {},
      // Empty space answers a right click too.
      child: ColoredBox(
        color: Colors.transparent,
        child: _showRing ? const Center(child: ProgressRing()) : child,
      ),
    );
  }

  void _clearResults() {
    setState(() {
      _searchResults = null;
      items = {};
      _showRing = false;
    });
  }

  // A debounced search that fires while another one runs is skipped. Once
  // that one is done, search for what the box says now.
  void _catchUp(String searched) {
    if (!mounted || (_debounce?.isActive ?? false)) return;

    final text = searchController.text;
    if (text != searched && text.trim().isNotEmpty) {
      _performSearch(text);
    }
  }

  void _setSelectedField((CollectionType, String Function(BuildContext)) item) {
    largeScreenLayoutManager.resetAnimations();

    setState(() {
      selectedItem = item;
    });

    playAnimations();
  }

  @override
  Widget build(BuildContext context) {
    final autoSuggestBox = SearchSuggestBox(
      key: _searchBoxKey,
      controller: searchController,
      searchResults: _searchResults,
      registerSearchTask: _registerSearchTask,
      deviceType: widget.deviceType,
    );

    if (widget.deviceType == DeviceType.tablet) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: mediumScreenLayoutManager,
        child: PageContentFrame(
          top: false,
          // The search box is at the top, below the status bar.
          builder: (context) => SafeArea(
            left: false,
            right: false,
            bottom: false,
            child: Column(
              children: [
                if (Platform.isWindows) SizedBox(height: 20),
                Padding(
                  padding: EdgeInsets.fromLTRB(
                    32 + context.keptClear.left,
                    20,
                    60 + context.keptClear.right,
                    20,
                  ),
                  child: autoSuggestBox,
                ),
                Expanded(
                  child: _resultsArea(
                    SingleChildScrollView(
                      keyboardDismissBehavior:
                          ScrollViewKeyboardDismissBehavior.onDrag,
                      padding: getScrollContainerPadding(context),
                      child: MediumScreenSearchTrackList(items: items),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      );
    }

    if (widget.deviceType == DeviceType.dock) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: bandScreenLayoutManager,
        child: PageContentFrame(
          builder: (context) => Column(
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 2),
                child: autoSuggestBox,
              ),
              const SizedBox(height: 2),
              Expanded(
                child: _resultsArea(
                  SingleChildScrollView(
                    keyboardDismissBehavior:
                        ScrollViewKeyboardDismissBehavior.onDrag,
                    padding: getScrollContainerPadding(context),
                    child: BandScreenSearchTrackList(
                      items: items,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }

    if (widget.deviceType == DeviceType.band) {
      return PageContentFrame(
        child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
          value: bandScreenLayoutManager,
          child: Row(
            children: [
              SizedBox(
                width: 120,
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 2),
                  child: autoSuggestBox,
                ),
              ),
              const SizedBox(width: 2),
              Expanded(
                child: _resultsArea(
                  SmoothHorizontalScroll(
                    builder: (context, controller) {
                      return SingleChildScrollView(
                        controller: controller,
                        keyboardDismissBehavior:
                            ScrollViewKeyboardDismissBehavior.onDrag,
                        scrollDirection: Axis.horizontal,
                        padding: getScrollContainerPadding(context),
                        child: BandScreenSearchTrackList(
                          items: items,
                          direction: Axis.horizontal,
                        ),
                      );
                    },
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }

    if (widget.deviceType == DeviceType.zune) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: smallScreenLayoutManager,
        child: PageContentFrame(
          builder: (context) => Column(
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: autoSuggestBox,
              ),
              Expanded(
                child: _resultsArea(
                  SingleChildScrollView(
                    keyboardDismissBehavior:
                        ScrollViewKeyboardDismissBehavior.onDrag,
                    padding: getScrollContainerPadding(context),
                    child: SmallScreenSearchTrackList(
                      items: items,
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: largeScreenLayoutManager,
      child: Row(
        children: [
          Expanded(
            child: PageContentFrame(
              top: false,
              left: false,
              right: false,
              // The sidebar has the right side of the window.
              child: SafeArea(
                right: false,
                bottom: false,
                child: _resultsArea(
                  LargeScreenSearchTrackList(
                    selectedItem: selectedItem,
                    items: items,
                  ),
                ),
              ),
            ),
          ),
          LargeScreenSearchSidebar(
            selectedItem: selectedItem,
            autoSuggestBox: autoSuggestBox,
            searchResults: _searchResults,
            setSelectedField: _setSelectedField,
            items: items,
          ),
        ],
      ),
    );
  }
}
