/// [items] in the order of [ids], each once. The database returns rows by
/// their id, not in the order they were asked for, which is the order that
/// matters, such as the ranking of search results.
List<T> orderByIds<T>(List<int> ids, List<T> items, int Function(T) idOf) {
  final byId = {for (final item in items) idOf(item): item};
  final seen = <int>{};
  return [
    for (final id in ids)
      if (seen.add(id) && byId.containsKey(id)) byId[id] as T,
  ];
}
