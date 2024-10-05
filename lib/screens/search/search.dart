import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:responsive_framework/responsive_framework.dart';

import '../../utils/api/search_for.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/api/fetch_media_file_by_ids.dart';
import '../../widgets/track_list/track_list.dart';
import '../../widgets/start_screen/start_screen.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../messages/search.pb.dart';
import '../../messages/collection.pb.dart';

import './widgets/search_suggest_box.dart';
import './widgets/large_screen_search_sidebar.dart';
import './widgets/large_screen_search_track_list.dart';

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});

  @override
  State<SearchPage> createState() => _SearchPageState();
}

class _SearchPageState extends State<SearchPage> {
  final searchController = TextEditingController();

  CollectionType selectedItem = CollectionType.Track;
  Timer? _debounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

  List<InternalMediaFile> tracks = [];
  List<InternalCollection> artists = [];
  List<InternalCollection> albums = [];
  List<InternalCollection> playlists = [];

  String _lastSearched = '';

  final layoutManager = StartScreenLayoutManager();

  @override
  void initState() {
    super.initState();

    searchController.addListener(_registerSearchTask);
  }

  @override
  void dispose() {
    searchController.dispose();
    _debounce?.cancel();
    layoutManager.dispose();
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
      layoutManager.resetAnimations();

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
        layoutManager.playAnimations();
      });
    } catch (e) {
      // Handle error
    } finally {
      setState(() {
        _isRequestInProgress = false;
      });
    }
  }

  void setSelectedField(CollectionType item) {
    setState(() {
      selectedItem = item;
    });
  }

  @override
  Widget build(BuildContext context) {
    final autoSuggestBox = SearchSuggestBox(
      controller: searchController,
      searchResults: _searchResults,
      registerSearchTask: _registerSearchTask,
    );

    final isMini = ResponsiveBreakpoints.of(context).smallerOrEqualTo(TABLET);

    if (isMini) {
      return ChangeNotifierProvider<StartScreenLayoutManager>.value(
        value: layoutManager,
        child: LargeScreenSearchTrackList(
          selectedItem: selectedItem,
          tracks: tracks,
          artists: artists,
          albums: albums,
          playlists: playlists,
        ),
      );
    }

    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: layoutManager,
      child: Row(
        children: [
          LargeScreenSearchTrackList(
            selectedItem: selectedItem,
            tracks: tracks,
            artists: artists,
            albums: albums,
            playlists: playlists,
          ),
          LargeScreenSearchSidebar(
            selectedItem: selectedItem,
            autoSuggestBox: autoSuggestBox,
            searchResults: _searchResults,
            setSelectedField: setSelectedField,
            tracks: tracks,
            artists: artists,
            albums: albums,
            playlists: playlists,
            layoutManager: layoutManager,
          ),
        ],
      ),
    );
  }
}
