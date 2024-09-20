import 'dart:async';
import 'package:fluent_ui/fluent_ui.dart';

class SearchTask<T, P> extends ChangeNotifier {
  P? _lastSearched;
  Timer? _debounce;

  bool notifyWhenStateChange;

  bool _isRequestInProgress = false;
  List<T> searchResults = [];
  final Future<List<T>> Function(P) searchDelegate;

  /// Creates a search task with the given delegate and state change notification option.
  SearchTask({
    required this.notifyWhenStateChange,
    required this.searchDelegate,
  });

  /// Registers a search task with a debounce mechanism.
  void search(P task) {
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
  Future<void> _performSearch(P query) async {
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
