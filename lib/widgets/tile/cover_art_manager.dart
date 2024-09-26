import 'dart:async';
import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/query_list.dart';
import '../../messages/cover_art.pb.dart';

class CoverArtCache {
  final Map<QueryList, Completer<List<int>>> _cache = {};

  Completer<List<int>>? get(QueryList query) {
    return _cache[query];
  }

  void set(QueryList query, Completer<List<int>> result) {
    _cache[query] = result;
  }

  void clear() {
    _cache.clear();
  }
}

class CoverArtManager with ChangeNotifier {
  final CoverArtCache _cache = CoverArtCache();
  final List<QueryList> _pendingQueries = [];
  Timer? _debounceTimer;

  Future<List<int>> queryCoverArts(QueryList query) async {
    // Check cache first
    final cachedResult = _cache.get(query);
    if (cachedResult != null) {
      return cachedResult.future;
    }

    // Add to pending queries
    _pendingQueries.add(query);

    // Debounce mechanism to batch requests
    _debounceTimer?.cancel();

    // Wait until the query is processed
    final completer = Completer<List<int>>();
    _pendingQueries.add(query);
    _debounceTimer = Timer(const Duration(milliseconds: 100), () async {
      final results = await _processQueries();
      completer.complete(results[query]);
    });

    _cache.set(query, completer);

    return completer.future;
  }

  _processQueries() async {
    final queriesToProcess = List<QueryList>.from(_pendingQueries);
    _pendingQueries.clear();

    // Create the request
    final requestUnits = queriesToProcess.map((query) {
      return GetCoverArtIdsByMixQueriesRequestUnit(
        id: query.hashCode,
        queries: query.toQueryList(),
      );
    }).toList();

    GetCoverArtIdsByMixQueriesRequest(
      requests: requestUnits,
    ).sendSignalToRust();

    final response =
        await GetCoverArtIdsByMixQueriesResponse.rustSignalStream.first;

    // Process the response
    for (final unit in response.message.result) {
      final query = queriesToProcess.firstWhere((q) => q.hashCode == unit.id);
      final coverArtIds = unit.coverArtIds;
      _cache.get(query)?.complete(coverArtIds);
    }

    notifyListeners();
  }

  @override
  void dispose() {
    _debounceTimer?.cancel();
    _pendingQueries.clear();
    _cache.clear();
    super.dispose();
  }
}
