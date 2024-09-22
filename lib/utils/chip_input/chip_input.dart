import 'dart:async';
import 'dart:collection';

import 'package:fluent_ui/fluent_ui.dart';

import './search_task.dart';
import './interactive_tag.dart';
import '../clear_text_utils/clear_text_controller.dart';
import '../clear_text_utils/clear_text_focus_node.dart';

class ChipInputController<T> extends ChangeNotifier {
  final LinkedHashSet<AutoSuggestBoxItem<T>> _selectedItems;

  ChipInputController([LinkedHashSet<AutoSuggestBoxItem<T>>? selectedItems])
      : _selectedItems =
            // ignore: prefer_collection_literals
            selectedItems ?? LinkedHashSet<AutoSuggestBoxItem<T>>();

  LinkedHashSet<AutoSuggestBoxItem<T>> get selectedItems => _selectedItems;

  void addItem(AutoSuggestBoxItem<T> item) {
    _selectedItems.add(item);
    notifyListeners();
  }

  void removeItem(AutoSuggestBoxItem<T> item) {
    _selectedItems.remove(item);
    notifyListeners();
  }

  void removeLastItem() {
    if (_selectedItems.isNotEmpty) {
      _selectedItems.remove(_selectedItems.last);
      notifyListeners();
    }
  }

  void clearItems() {
    _selectedItems.clear();
    notifyListeners();
  }

  @override
  void dispose() {
    super.dispose();
    _selectedItems.clear();
  }
}

class ChipInput<T> extends StatefulWidget {
  final Future<List<AutoSuggestBoxItem<T>>> Function() getInitResult;
  final Future<List<AutoSuggestBoxItem<T>>> Function(String query) searchFor;
  final ChipInputController<T>? controller;

  const ChipInput({
    super.key,
    required this.getInitResult,
    required this.searchFor,
    this.controller,
  });

  @override
  State<ChipInput> createState() => _ChipInputState<T>();
}

class _ChipInputState<T> extends State<ChipInput<T>> {
  late SearchTask<AutoSuggestBoxItem<T>, String> _searcher;
  List<AutoSuggestBoxItem<T>> searchResults = [];
  late ChipInputController<T> _controller;

  late DeleteDetectingController _textController;
  late DeleteDetectingFocusNode _focusNode;
  late final Future<List<AutoSuggestBoxItem<T>>> _initResult;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? ChipInputController<T>();
    _controller.addListener(_updateSelectedItems);

    _textController = DeleteDetectingController();
    _focusNode = DeleteDetectingFocusNode(_textController, false);

    _textController.addListener(_search);
    _textController.isTextClearedNotifier.addListener(_handleTextCleared);

    _initResult = widget.getInitResult();

    _searcher = SearchTask<AutoSuggestBoxItem<T>, String>(
      notifyWhenStateChange: false,
      searchDelegate: (query) async {
        return await widget.searchFor(query);
      },
    );

    _searcher.addListener(() {
      setState(() {
        searchResults = _searcher.searchResults;
      });
    });
  }

  void _search() async {
    if (_textController.clearText.isEmpty) {
      final initResult = await _initResult;

      if (searchResults == initResult) return;
      if (mounted) {
        setState(() {
          searchResults = initResult;
        });
      }
    } else {
      _searcher.search(_textController.clearText);
    }
  }

  void _handleTextCleared() {
    if (_textController.isTextClearedNotifier.value) {
      setState(() {
        _controller.removeLastItem();
      });
    }
  }

  void _updateSelectedItems() {
    setState(() {});
  }

  @override
  void dispose() {
    _searcher.dispose();
    _focusNode.dispose();
    _textController.dispose();
    if (widget.controller == null) {
      _controller.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AutoSuggestBox(
      controller: _textController,
      focusNode: _focusNode,
      items: searchResults,
      onSelectTextDelegate: (item) => '\u200B',
      onSelected: (item) {
        setState(() {
          _controller.addItem(item);
        });
      },
      sorter: (text, items) => items,
      leadingIcon: Padding(
        padding: EdgeInsets.fromLTRB(
          4.0,
          _controller.selectedItems.isEmpty ? 0.0 : 4.0,
          4.0,
          0.0,
        ),
        child: Wrap(
          spacing: 4.0,
          runSpacing: 4.0,
          children: _controller.selectedItems.map(
            (item) {
              return InteractiveTag(
                child: Text(item.label),
                onPressed: () {
                  setState(() {
                    _controller.removeItem(item);
                  });
                },
              );
            },
          ).toList(),
        ),
      ),
      decorationBuilder: (context, body, prefix, suffix) {
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [prefix ?? Container(), body],
        );
      },
    );
  }
}
