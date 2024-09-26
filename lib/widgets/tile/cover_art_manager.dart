import 'dart:async';

import '../../utils/query_list.dart';
import '../../messages/cover_art.pb.dart';

class CoverArtCache {
  final Map<QueryList, List<int>> _caches = {};
  final Map<QueryList, Completer<List<int>>> _completers = {};

  List<int>? getCache(QueryList query) {
    return _caches[query];
  }

  Completer<List<int>>? getCompleter(QueryList query) {
    return _completers[query];
  }

  void registerCompleter(QueryList query, Completer<List<int>> result) {
    _completers[query] = result;
  }

  void registerCache(QueryList query, List<int> result) {
    _caches[query] = result;
  }

  void clear() {
    _completers.clear();
    _caches.clear();
  }
}

class CoverArtManager {
  final CoverArtCache _cache = CoverArtCache();

  final Set _pendingQueries = {};

  Future<void> commit() async {
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

      _cache.registerCache(query, coverArtIds);
      final task = _cache.getCompleter(query);

      if (task == null) return;
      if (task.isCompleted) return;

      task.complete(coverArtIds);
    }
  }

  Future<List<int>> queryCoverArts(QueryList query) async {
    // Check cache first
    final cachedResult = _cache.getCompleter(query);
    if (cachedResult != null) {
      return cachedResult.future;
    }

    // Wait until the query is processed
    final completer = Completer<List<int>>();

    _cache.registerCompleter(query, completer);
    _pendingQueries.add(query);

    return completer.future;
  }

  List<int>? getResult(QueryList query) {
    return _cache.getCache(query);
  }
}
