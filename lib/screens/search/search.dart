import 'dart:async';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';
import 'package:get_storage/get_storage.dart';

import '../../messages/search.pb.dart';

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

  String selectedItem = '';
  Timer? _debounce;
  Timer? _saveDebounce;
  bool _isRequestInProgress = false;
  SearchForResponse? _searchResults;

  final box = GetStorage();
  List<String> suggestions = [];

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
      if (_debounce?.isActive ?? false) _debounce!.cancel();
      _debounce = Timer(const Duration(milliseconds: 300), () {
        if (!_isRequestInProgress) {
          _performSearch(searchController.text);
        }
      });

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
    super.dispose();
  }

  Future<void> _performSearch(String query) async {
    if (_isRequestInProgress) return;
    setState(() {
      _isRequestInProgress = true;
    });

    try {
      final response = await searchFor(query);
      setState(() {
        _searchResults = response;
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
            _performSearch(suggestion);
          },
        );
      }).toList(),
      trailingIcon: IgnorePointer(
        child: IconButton(
          onPressed: () {},
          icon: const Icon(FluentIcons.search),
        ),
      ),
    );

    return Row(children: [
      Expanded(
        child: Column(
          children: [
            if (_searchResults != null) ...[
              Text('Artists: ${_searchResults!.artists.join(", ")}'),
              Text('Albums: ${_searchResults!.albums.join(", ")}'),
              Text('Playlists: ${_searchResults!.playlists.join(", ")}'),
              Text('Tracks: ${_searchResults!.tracks.join(", ")}'),
            ],
          ],
        ),
      ),
      SizedBox(
        width: 320,
        child: Container(
          color: theme.cardColor,
          child: Padding(
            padding: const EdgeInsets.all(12),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.start,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Padding(
                  padding:
                      const EdgeInsets.symmetric(vertical: 13, horizontal: 16),
                  child: Text("Search", style: typography.bodyLarge),
                ),
                Padding(
                  padding:
                      const EdgeInsets.symmetric(vertical: 8, horizontal: 16),
                  child: SizedBox(
                    height: 36,
                    child: Row(
                      mainAxisSize: MainAxisSize.max,
                      children: [Flexible(fit: FlexFit.loose, child: search)],
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
                            itemCount = _searchResults!.artists.length;
                            break;
                          case 'Albums':
                            itemCount = _searchResults!.albums.length;
                            break;
                          case 'Playlists':
                            itemCount = _searchResults!.playlists.length;
                            break;
                          case 'Tracks':
                            itemCount = _searchResults!.tracks.length;
                            break;
                        }
                      }
                      return ListTile.selectable(
                        leading: SizedBox(
                          height: 36,
                          child: AspectRatio(
                            aspectRatio: 1,
                            child: ColoredBox(
                              color: theme.accentColor,
                              child: Icon(searchIcons[index], size: 26),
                            ),
                          ),
                        ),
                        title: Row(
                          children: [
                            Expanded(child: Text(item, style: typography.body)),
                            if (itemCount > 0)
                              Text(
                                '$itemCount',
                                style: typography.body!.copyWith(
                                  color: theme.activeColor.withAlpha(160),
                                ),
                              ),
                          ],
                        ),
                        selectionMode: ListTileSelectionMode.single,
                        selected: selectedItem == item,
                        onSelectionChange: (v) =>
                            setState(() => selectedItem = item),
                      );
                    },
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    ]);
  }
}

Future<SearchForResponse> searchFor(String query) async {
  final searchRequest = SearchForRequest(queryStr: query, n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  return (await SearchForResponse.rustSignalStream.first).message;
}
