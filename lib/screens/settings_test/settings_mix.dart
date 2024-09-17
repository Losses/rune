import 'dart:async';
import 'dart:collection';

import 'package:fluent_ui/fluent_ui.dart';

import '../../messages/artist.pb.dart';
import '../../messages/search.pb.dart';

import '../../utils/clear_text_utils/clear_text_controller.dart';
import '../../utils/clear_text_utils/clear_text_focus_node.dart';
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
              content: const Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text('Artists'),
                  SearchInput(field: 'artist'),
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
  late SearchTask<AutoSuggestBoxItem<int>> _searcher;
  List<AutoSuggestBoxItem<int>> searchResults = [];
  LinkedHashSet<AutoSuggestBoxItem<int>> selectedItems = LinkedHashSet();

  late DeleteDetectingController _controller;
  late DeleteDetectingFocusNode _focusNode;
  late final Future<List<AutoSuggestBoxItem<int>>> _initResult =
      getInitResult();

  Future<List<AutoSuggestBoxItem<int>>> getInitResult() async {
    SearchArtistSummaryRequest(n: 50).sendSignalToRust();
    return (await SearchArtistSummaryResponse.rustSignalStream.first)
        .message
        .result
        .map((x) => AutoSuggestBoxItem<int>(value: x.id, label: x.name))
        .toList();
  }

  @override
  void initState() {
    _controller = DeleteDetectingController();
    _focusNode = DeleteDetectingFocusNode(_controller, false);
    super.initState();
    _controller.addListener(_search);
    _controller.isTextClearedNotifier.addListener(_handleTextCleared);

    _searcher = SearchTask<AutoSuggestBoxItem<int>>(
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

    _searcher.addListener(() {
      setState(() {
        searchResults = _searcher.searchResults;
      });
    });
  }

  void _search() async {
    if (_controller.clearText.isEmpty) {
      final initResult = await _initResult;

      if (searchResults == initResult) return;
      if (mounted) {
        setState(() {
          searchResults = initResult;
        });
      }
    } else {
      _searcher.search(_controller.clearText);
    }
  }

  void _handleTextCleared() {
    if (_controller.isTextClearedNotifier.value) {
      setState(() {
        final list = selectedItems.toList();

        if (list.isNotEmpty) {
          list.removeLast();
        }

        selectedItems = LinkedHashSet.from(list);
      });
    }
  }

  @override
  void dispose() {
    super.dispose();
    _searcher.dispose();
    _focusNode.dispose();
    _controller.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AutoSuggestBox(
      controller: _controller,
      focusNode: _focusNode,
      items: searchResults,
      onSelectTextDelegate: (item) => '\u200B',
      onSelected: (item) {
        setState(() {
          selectedItems.add(item);
        });
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
