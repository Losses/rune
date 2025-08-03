import 'dart:io';
import 'dart:async';

import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/api/search_for.dart';
import '../../utils/api/fetch_collection_by_ids.dart';
import '../../utils/api/fetch_media_file_by_ids.dart';
import '../../widgets/smooth_horizontal_scroll.dart';
import '../../widgets/start_screen/utils/internal_collection.dart';
import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/page_content_frame.dart';
import '../../screens/search/widgets/small_screen_search_track_list.dart';
import '../../screens/search/constants/search_categories.dart';
import '../../bindings/bindings.dart';
import '../../providers/responsive_providers.dart';

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

  (CollectionType, String Function(BuildContext)) selectedItem =
      searchCategories[0];
  Timer? _debounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

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
  }

  @override
  void dispose() {
    super.dispose();
    _debounce?.cancel();
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
        items[CollectionType.track] = trackItemsToSearchCard(
          await fetchMediaFileByIds(response.tracks, true),
        );
      }
      if (response.artists.isNotEmpty) {
        items[CollectionType.artist] = await collectionResponseToSearchCard(
          response.artists,
          CollectionType.artist,
        );
      }
      if (response.albums.isNotEmpty) {
        items[CollectionType.album] = await collectionResponseToSearchCard(
          response.albums,
          CollectionType.album,
        );
      }
      if (response.playlists.isNotEmpty) {
        items[CollectionType.playlist] = await collectionResponseToSearchCard(
          response.playlists,
          CollectionType.playlist,
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
      controller: searchController,
      searchResults: _searchResults,
      registerSearchTask: _registerSearchTask,
      deviceType: widget.deviceType,
    );

    final viewPadding = MediaQuery.of(context).viewPadding;

    if (widget.deviceType == DeviceType.tablet) {
      return PageContentFrame(
        top: false,
        child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
          value: mediumScreenLayoutManager,
          child: Column(
            children: [
              if (Platform.isWindows) SizedBox(height: 20),
              Padding(
                padding: EdgeInsets.fromLTRB(
                  32 + viewPadding.left,
                  20 + viewPadding.top,
                  60 + viewPadding.right,
                  20 + viewPadding.bottom,
                ),
                child: autoSuggestBox,
              ),
              Expanded(
                child: SingleChildScrollView(
                  padding: getScrollContainerPadding(context, top: false),
                  child: MediumScreenSearchTrackList(
                    items: items,
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    }

    if (widget.deviceType == DeviceType.dock) {
      return PageContentFrame(
        child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
          value: bandScreenLayoutManager,
          child: Column(
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 2),
                child: autoSuggestBox,
              ),
              const SizedBox(height: 2),
              Expanded(
                child: SingleChildScrollView(
                  padding: getScrollContainerPadding(context),
                  child: BandScreenSearchTrackList(
                    items: items,
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
                child: SmoothHorizontalScroll(
                  builder: (context, controller) {
                    return SingleChildScrollView(
                      controller: controller,
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
            ],
          ),
        ),
      );
    }

    if (widget.deviceType == DeviceType.zune) {
      return PageContentFrame(
        child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
          value: smallScreenLayoutManager,
          child: Column(
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 12),
                child: autoSuggestBox,
              ),
              Expanded(
                child: SingleChildScrollView(
                  padding: getScrollContainerPadding(context),
                  child: SmallScreenSearchTrackList(
                    items: items,
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
              child: LargeScreenSearchTrackList(
                selectedItem: selectedItem,
                items: items,
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
