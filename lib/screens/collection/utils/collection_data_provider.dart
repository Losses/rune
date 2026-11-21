import 'dart:async';
import 'dart:math';

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

  final Future<List<Group<InternalCollection>>> Function(CollectionType)
  _loadSummary;
  final Future<(List<Group<InternalCollection>>, bool)> Function(
    CollectionType,
    int,
    int,
  )
  _loadPage;

  // The in-flight reload, if any, and the generation that owns it.
  Future<void>? _reloading;
  int _reloadGeneration = -1;

  CollectionDataProvider({
    required this.collectionType,
    Stream<void>? contentChanged,
    Future<List<Group<InternalCollection>>> Function(CollectionType)?
    loadSummary,
    Future<(List<Group<InternalCollection>>, bool)> Function(
      CollectionType,
      int,
      int,
    )?
    loadPage,
  }) : _loadSummary = loadSummary ?? fetchCollectionPageSummary,
       _loadPage = loadPage ?? fetchCollectionPagePage {
    final changes =
        contentChanged ??
        LibraryContentChanged.rustSignalStream.map<void>((_) {});
    _contentSubscription = changes.listen((_) {
      unawaited(reloadData());
    });
  }

  @override
  void dispose() {
    _disposed = true;
    _generation++;
    _contentSubscription.cancel();
    super.dispose();
  }

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

    final generation = _generation;
    final cacheGeneration = _cache.generation;
    final data = await _loadSummary(collectionType);
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
    final (newItems, isLastPage) = await _loadPage(
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

  Future<void> fetchData() async {
    // Appending while a reload runs would duplicate pages, so wait for it.
    final reloading = _reloading;
    if (reloading != null) return reloading;

    initialized = true;
    isLoading = true;

    notifyListeners();

    final generation = _generation;
    final thisCursor = cursor;
    cursor += 1;
    final (newItems, newIsLastPage) = await _fetchPage(thisCursor);

    if (_disposed || generation != _generation) return;
    isLoading = false;
    isLastPage = newIsLastPage;
    items.addAll(newItems);

    notifyListeners();
  }

  Future<void> reloadData() {
    _generation++;
    _cache.clearType(collectionType);
    summary = _fetchSummaryWithCache();
    notifyListeners();
    final generation = _generation;
    _reloadGeneration = generation;
    return _reloading = _reload(generation);
  }

  // Keeps the old items on screen until the fresh pages have all arrived,
  // so a scrolled list does not jump back to the start.
  Future<void> _reload(int generation) async {
    try {
      final pages = max(cursor, 1);
      final fresh = <Group<InternalCollection>>[];
      var fetched = 0;
      var last = false;
      while (fetched < pages && !last) {
        final (newItems, newIsLastPage) = await _fetchPage(fetched);
        if (_disposed || generation != _generation) return;
        fresh.addAll(newItems);
        fetched += 1;
        last = newIsLastPage;
      }
      items = fresh;
      cursor = fetched;
      isLastPage = last;
      isLoading = false;
      initialized = true;
      notifyListeners();
    } finally {
      if (_reloadGeneration == generation) _reloading = null;
    }
  }

  static void clearAllCache() {
    CollectionCache().clearAll();
  }
}
