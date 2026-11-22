import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../../widgets/start_screen/utils/group.dart';
import '../../../widgets/start_screen/utils/internal_collection.dart';
import '../../../bindings/bindings.dart';

import 'fetch_page.dart';
import 'fetch_groups.dart';
import 'fetch_summary.dart';

class CollectionCache {
  static final CollectionCache _instance = CollectionCache._internal();

  factory CollectionCache() {
    return _instance;
  }

  CollectionCache._internal();

  int _generation = 0;
  int get generation => _generation;

  final _summaryCache = <CollectionType, List<Group<InternalCollection>>>{};
  final _pageCache =
      <CollectionType, Map<int, List<Group<InternalCollection>>>>{};
  final _groupCache =
      <CollectionType, Map<String, List<Group<InternalCollection>>>>{};

  void cacheSummary(CollectionType type, List<Group<InternalCollection>> data) {
    _summaryCache[type] = data;
  }

  void cachePage(
    CollectionType type,
    int cursor,
    List<Group<InternalCollection>> data,
  ) {
    if (!_pageCache.containsKey(type)) {
      _pageCache[type] = {};
    }
    _pageCache[type]![cursor] = data;
  }

  void cacheGroups(
    CollectionType type,
    String groupKey,
    List<Group<InternalCollection>> data,
  ) {
    if (!_groupCache.containsKey(type)) {
      _groupCache[type] = {};
    }
    _groupCache[type]![groupKey] = data;
  }

  List<Group<InternalCollection>>? getSummary(CollectionType type) {
    return _summaryCache[type];
  }

  List<Group<InternalCollection>>? getPage(CollectionType type, int cursor) {
    return _pageCache[type]?[cursor];
  }

  List<Group<InternalCollection>>? getGroups(
    CollectionType type,
    String groupKey,
  ) {
    return _groupCache[type]?[groupKey];
  }

  void clearType(CollectionType type) {
    _generation++;
    _summaryCache.remove(type);
    _pageCache.remove(type);
    _groupCache.remove(type);
  }

  void clearAll() {
    _generation++;
    _summaryCache.clear();
    _pageCache.clear();
    _groupCache.clear();
  }
}

class CollectionDataProvider with ChangeNotifier {
  static const _pageSize = 3;
  final CollectionType collectionType;
  final _cache = CollectionCache();

  late final StreamSubscription _contentSubscription;
  int _generation = 0;
  bool _disposed = false;

  CollectionDataProvider({required this.collectionType}) {
    _contentSubscription = LibraryContentChanged.rustSignalStream.listen((_) {
      unawaited(reloadData());
    });
  }

  @override
  void dispose() {
    _disposed = true;
    _generation++;
    _pageRequest = null;
    _contentSubscription.cancel();
    super.dispose();
  }

  late Future<List<Group<InternalCollection>>> summary =
      _fetchSummaryWithCache();

  List<Group<InternalCollection>> items = [];

  Future<void>? _pageRequest;
  bool get isLoading => _pageRequest != null;

  /// The failure for the current page. Starting a retry clears it.
  Object? pageError;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  Future<List<Group<InternalCollection>>> _fetchSummaryWithCache() async {
    final cachedSummary = _cache.getSummary(collectionType);
    if (cachedSummary != null) {
      return cachedSummary;
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final data = await fetchCollectionPageSummary(collectionType);
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cacheSummary(collectionType, data);
    }
    return data;
  }

  Future<(List<Group<InternalCollection>>, bool)> _fetchPage(int cursor) async {
    final cachedPage = _cache.getPage(collectionType, cursor);
    if (cachedPage != null) {
      return (cachedPage, cachedPage.length < _pageSize);
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final (newItems, isLastPage) = await fetchCollectionPagePage(
      collectionType,
      _pageSize,
      cursor,
    );
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cachePage(collectionType, cursor, newItems);
    }
    return (newItems, isLastPage);
  }

  Future<List<Group<InternalCollection>>> fetchGroups(
    List<String> groupTitles,
  ) async {
    // For simplicity, use concatenated groupTitles as the cache key
    final groupKey = groupTitles.join(',');
    final cachedGroups = _cache.getGroups(collectionType, groupKey);
    if (cachedGroups != null) {
      return cachedGroups;
    }

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final data = await fetchCollectionPageGroups(collectionType, groupTitles);
    if (generation == _generation &&
        cacheGeneration == _cache.generation &&
        !_disposed) {
      _cache.cacheGroups(collectionType, groupKey, data);
    }
    return data;
  }

  Future<void> fetchData() {
    final active = _pageRequest;
    if (active != null) return active;
    if (_disposed || isLastPage) return Future.value();
    initialized = true;
    pageError = null;
    final request = _loadPage(_generation, cursor);
    _pageRequest = request;
    notifyListeners();
    return request;
  }

  Future<void> _loadPage(int generation, int page) async {
    try {
      final (newItems, newIsLastPage) = await _fetchPage(page);
      if (_disposed || generation != _generation) return;
      items.addAll(newItems);
      cursor = page + 1;
      isLastPage = newIsLastPage;
    } catch (error) {
      if (!_disposed && generation == _generation) pageError = error;
    } finally {
      if (!_disposed && generation == _generation) {
        _pageRequest = null;
        notifyListeners();
      }
    }
  }

  Future<void> reloadData() async {
    if (_disposed) return;
    _generation++;
    _pageRequest = null;
    _cache.clearType(collectionType);
    summary = _fetchSummaryWithCache();
    cursor = 0;
    isLastPage = false;
    items = [];
    await fetchData();
  }

  static void clearAllCache() {
    CollectionCache().clearAll();
  }
}
