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

  final _summaryCache = <CollectionType, List<Group<InternalCollection>>>{};
  final _pageCache =
      <CollectionType, Map<int, List<Group<InternalCollection>>>>{};
  final _groupCache =
      <CollectionType, Map<String, List<Group<InternalCollection>>>>{};

  void cacheSummary(CollectionType type, List<Group<InternalCollection>> data) {
    _summaryCache[type] = data;
  }

  void cachePage(
      CollectionType type, int cursor, List<Group<InternalCollection>> data) {
    if (!_pageCache.containsKey(type)) {
      _pageCache[type] = {};
    }
    _pageCache[type]![cursor] = data;
  }

  void cacheGroups(CollectionType type, String groupKey,
      List<Group<InternalCollection>> data) {
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
      CollectionType type, String groupKey) {
    return _groupCache[type]?[groupKey];
  }

  void clearType(CollectionType type) {
    _summaryCache.remove(type);
    _pageCache.remove(type);
    _groupCache.remove(type);
  }

  void clearAll() {
    _summaryCache.clear();
    _pageCache.clear();
    _groupCache.clear();
  }
}

class CollectionDataProvider with ChangeNotifier {
  static const _pageSize = 3;
  final CollectionType collectionType;
  final _cache = CollectionCache();

  CollectionDataProvider({required this.collectionType});

  late Future<List<Group<InternalCollection>>> summary =
      _fetchSummaryWithCache();

  List<Group<InternalCollection>> items = [];

  bool isLoading = false;
  bool isLastPage = false;
  bool initialized = false;
  int cursor = 0;

  Future<List<Group<InternalCollection>>> _fetchSummaryWithCache() async {
    final cachedSummary = _cache.getSummary(collectionType);
    if (cachedSummary != null) {
      return cachedSummary;
    }

    final data = await fetchCollectionPageSummary(collectionType);
    _cache.cacheSummary(collectionType, data);
    return data;
  }

  Future<(List<Group<InternalCollection>>, bool)> _fetchPage(
    int cursor,
  ) async {
    final cachedPage = _cache.getPage(collectionType, cursor);
    if (cachedPage != null) {
      return (cachedPage, cachedPage.length < _pageSize);
    }

    final (newItems, isLastPage) =
        await fetchCollectionPagePage(collectionType, _pageSize, cursor);
    _cache.cachePage(collectionType, cursor, newItems);
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

    final data = await fetchCollectionPageGroups(collectionType, groupTitles);
    _cache.cacheGroups(collectionType, groupKey, data);
    return data;
  }

  Future<void> fetchData() async {
    initialized = true;
    isLoading = true;

    notifyListeners();

    final thisCursor = cursor;
    cursor += 1;
    final (newItems, newIsLastPage) = await _fetchPage(thisCursor);

    isLoading = false;
    isLastPage = newIsLastPage;
    items.addAll(newItems);

    notifyListeners();
  }

  Future<void> reloadData() async {
    _cache.clearType(collectionType);
    cursor = 0;
    items = [];
    notifyListeners();
    await fetchData();
    notifyListeners();
  }

  static void clearAllCache() {
    CollectionCache().clearAll();
  }
}
