import 'dart:async';

import 'package:player/screens/search/widgets/small_screen_search_track_list.dart';
import 'package:player/widgets/navigation_bar/navigation_bar_placeholder.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/api/search_for.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/api/fetch_media_file_by_ids.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../messages/search.pb.dart';
import '../../messages/collection.pb.dart';
import '../../providers/responsive_providers.dart';

import 'utils/track_items_to_search_card.dart';
import 'utils/collection_items_to_search_card.dart';
import 'widgets/search_card.dart';
import 'widgets/search_suggest_box.dart';
import 'widgets/collection_search_item.dart';
import 'widgets/large_screen_search_sidebar.dart';
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
    return BreakpointBuilder(
      breakpoints: const [DeviceType.zune, DeviceType.tablet, DeviceType.tv],
      builder: (context, deviceType) {
        return SearchPageImplementation(
          deviceType: deviceType,
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

  CollectionType selectedItem = CollectionType.Track;
  Timer? _debounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

  Map<CollectionType, List<SearchCard>> items = {};

  String _lastSearched = '';

  final largeScreenLayoutManager = StartScreenLayoutManager();
  final mediumScreenLayoutManager = StartScreenLayoutManager();
  final smallScreenLayoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();

    searchController.addListener(_registerSearchTask);
  }

  @override
  void dispose() {
    super.dispose();
    _debounce?.cancel();
    searchController.dispose();
    largeScreenLayoutManager.dispose();
    mediumScreenLayoutManager.dispose();
    smallScreenLayoutManager.dispose();
  }

  void resetAnimations() {
    largeScreenLayoutManager.resetAnimations();
    mediumScreenLayoutManager.resetAnimations();
    smallScreenLayoutManager.resetAnimations();
  }

  void playAnimations() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
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
    _debounce = Timer(const Duration(milliseconds: 300), () {
      if (!_isRequestInProgress) {
        _performSearch(searchController.text);
      }
    });
  }

  Future<List<CollectionSearchItem>> collectionResponseToSearchCard(
    List<int> ids,
    CollectionType type,
  ) async {
    return collectionItemsToSearchCard(
      (await fetchCollectionByIds(type, ids))
          .map(InternalCollection.fromRawCollection)
          .toList(),
      type,
    );
  }

  Future<void> _performSearch(String query) async {
    if (_isRequestInProgress) return;
    _isRequestInProgress = true;

    try {
      final response = await searchFor(query);
      setState(() {
        _searchResults = response;
      });

      items = {};

      if (response.tracks.isNotEmpty) {
        items[CollectionType.Track] = trackItemsToSearchCard(
          await fetchMediaFileByIds(response.tracks, true),
        );
      }
      if (response.artists.isNotEmpty) {
        items[CollectionType.Artist] = await collectionResponseToSearchCard(
          response.artists,
          CollectionType.Artist,
        );
      }
      if (response.albums.isNotEmpty) {
        items[CollectionType.Album] = await collectionResponseToSearchCard(
          response.albums,
          CollectionType.Album,
        );
      }
      if (response.playlists.isNotEmpty) {
        items[CollectionType.Playlist] = await collectionResponseToSearchCard(
          response.playlists,
          CollectionType.Playlist,
        );
      }

      setState(() {
        resetAnimations();
        playAnimations();
      });
    } catch (e) {
      // Handle error
    } finally {
      _isRequestInProgress = false;
    }
  }

  void _setSelectedField(CollectionType item) {
    largeScreenLayoutManager.resetAnimations();

    setState(() {
      selectedItem = item;
    });

    playAnimations();
  }

  @override
  Widget build(BuildContext context) {
    final autoSuggestBox = SearchSuggestBox(
      controller: searchController,
      searchResults: _searchResults,
      registerSearchTask: _registerSearchTask,
      deviceType: widget.deviceType,
    );

    if (widget.deviceType == DeviceType.tablet) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: mediumScreenLayoutManager,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(32, 18, 64, 20),
              child: autoSuggestBox,
            ),
            Expanded(
              child: SingleChildScrollView(
                child: MediumScreenSearchTrackList(
                  items: items,
                ),
              ),
            ),
            const PlaybackPlaceholder(),
          ],
        ),
      );
    }

    if (widget.deviceType == DeviceType.zune) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: smallScreenLayoutManager,
        child: Column(
          children: [
            const NavigationBarPlaceholder(),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: autoSuggestBox,
            ),
            Expanded(
              child: SingleChildScrollView(
                child: SmallScreenSearchTrackList(
                  items: items,
                ),
              ),
            ),
            const PlaybackPlaceholder(),
          ],
        ),
      );
    }

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: largeScreenLayoutManager,
      child: Row(
        children: [
          Expanded(
            child: LargeScreenSearchTrackList(
              selectedItem: selectedItem,
              items: items,
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
