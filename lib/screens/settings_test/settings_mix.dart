import 'dart:async';
import 'dart:collection';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/messages/artist.pb.dart';
import 'package:player/messages/search.pb.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class InteractiveTag extends BaseButton {
  /// Creates a button.
  const InteractiveTag({
    super.key,
    required super.child,
    required super.onPressed,
    super.onLongPress,
    super.onTapDown,
    super.onTapUp,
    super.focusNode,
    super.autofocus = false,
    super.style,
    super.focusable = true,
  });

  @override
  ButtonStyle defaultStyleOf(BuildContext context) {
    assert(debugCheckHasFluentTheme(context));
    final theme = FluentTheme.of(context);
    return ButtonStyle(
      shadowColor: WidgetStatePropertyAll(theme.shadowColor),
      padding: const WidgetStatePropertyAll(kDefaultButtonPadding),
      shape: WidgetStatePropertyAll(
        RoundedRectangleBorder(borderRadius: BorderRadius.circular(4.0)),
      ),
      backgroundColor: WidgetStateProperty.resolveWith((states) {
        return ButtonThemeData.buttonColor(context, states);
      }),
      foregroundColor: WidgetStateProperty.resolveWith((states) {
        return ButtonThemeData.buttonForegroundColor(context, states);
      }),
    );
  }

  @override
  ButtonStyle? themeStyleOf(BuildContext context) {
    final typography = FluentTheme.of(context).typography;
    assert(debugCheckHasFluentTheme(context));

    return ButtonStyle(textStyle: WidgetStateProperty.all(typography.caption));
  }
}

class SearchTask<T> extends ChangeNotifier {
  String _lastSearched = '';
  Timer? _debounce;

  bool notifyWhenStateChange;

  bool _isRequestInProgress = false;
  List<T> searchResults = [];
  final Future<List<T>> Function(String) searchDelegate;

  /// Creates a search task with the given delegate and state change notification option.
  SearchTask({
    required this.notifyWhenStateChange,
    required this.searchDelegate,
  });

  /// Registers a search task with a debounce mechanism.
  void search(String task) {
    if (_lastSearched == task) return;

    _lastSearched = task;
    _debounce?.cancel();
    _debounce = Timer(const Duration(milliseconds: 300), () {
      if (!_isRequestInProgress) {
        _performSearch(task);
      }
    });
  }

  /// Performs the search using the search delegate.
  Future<void> _performSearch(String query) async {
    if (_isRequestInProgress) return;
    _isRequestInProgress = true;

    if (notifyWhenStateChange) {
      notifyListeners();
    }

    try {
      final response = await searchDelegate(query);
      searchResults = response;
    } catch (e) {
      searchResults = [];
    } finally {
      _isRequestInProgress = false;
      notifyListeners();
    }
  }

  @override
  void dispose() {
    _debounce?.cancel();
    super.dispose();
  }
}

Future<SearchForResponse> searchFor(String query, String field) async {
  final searchRequest =
      SearchForRequest(queryStr: query, fields: [field], n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  return (await SearchForResponse.rustSignalStream.first).message;
}

class SettingsMixPage extends StatefulWidget {
  const SettingsMixPage({super.key});

  @override
  State<SettingsMixPage> createState() => _SettingsMixPageState();
}

class _SettingsMixPageState extends State<SettingsMixPage> {
  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            ContentDialog(
              title: const Column(
                children: [
                  SizedBox(height: 8),
                  Text("Create Mix"),
                ],
              ),
              content: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'Artists',
                    style: typography.bodyStrong,
                  ),
                  const SearchInput(
                    field: 'artist',
                  ),
                  Text(
                    'Albums',
                    style: typography.bodyStrong,
                  ),
                ],
              ),
              actions: [
                Button(
                  child: const Text('Delete'),
                  onPressed: () {
                    Navigator.pop(context, 'User deleted file');
                    // Delete file here
                  },
                ),
                FilledButton(
                  child: const Text('Cancel'),
                  onPressed: () =>
                      Navigator.pop(context, 'User canceled dialog'),
                ),
              ],
            )
          ],
        ),
      )
    ]);
  }
}

class SearchInput extends StatefulWidget {
  final String field;

  const SearchInput({
    super.key,
    required this.field,
  });

  @override
  State<SearchInput> createState() => _SearchInputState();
}

class _SearchInputState extends State<SearchInput> {
  late SearchTask<AutoSuggestBoxItem<int>> searcher;
  List<AutoSuggestBoxItem<int>> searchResults = [];
  LinkedHashSet<AutoSuggestBoxItem<int>> selectedItems = LinkedHashSet();

  final searchFocusNode = FocusNode();
  final searchController = TextEditingController();

  @override
  void initState() {
    super.initState();
    searcher = SearchTask<AutoSuggestBoxItem<int>>(
        notifyWhenStateChange: false,
        searchDelegate: (query) async {
          final ids = (await searchFor(query, widget.field)).artists;

          FetchArtistsByIdsRequest(ids: ids).sendSignalToRust();
          return (await FetchArtistsByIdsResponse.rustSignalStream.first)
              .message
              .result
              .map((x) => AutoSuggestBoxItem<int>(value: x.id, label: x.name))
              .toList();
        });

    searcher.addListener(() {
      setState(() {
        searchResults = searcher.searchResults;
      });
    });
  }

  @override
  void dispose() {
    super.dispose();
    searcher.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AutoSuggestBox(
      controller: searchController,
      focusNode: searchFocusNode,
      items: searchResults,
      onSelectedBehavior: OnSelectBehavior.clear,
      onChanged: (text, reason) {
        searcher.search(text);
      },
      onSelected: (item) {
        selectedItems.add(item);
      },
      sorter: (text, items) => items,
      leadingIcon: Padding(
        padding: EdgeInsets.fromLTRB(
            4.0, selectedItems.isEmpty ? 0.0 : 4.0, 4.0, 0.0),
        child: Wrap(
          spacing: 4.0,
          runSpacing: 4.0,
          children: selectedItems.map((item) {
            return InteractiveTag(
              child: Text(item.label),
              onPressed: () {
                setState(() {
                  selectedItems.remove(item);
                });
              },
            );
          }).toList(),
        ),
      ),
      decorationBuilder: (context, body, prefix, suffix) {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            prefix ?? Container(),
            Row(
              children: [
                Expanded(child: body),
                suffix ?? Container(),
              ],
            )
          ],
        );
      },
    );
  }
}
