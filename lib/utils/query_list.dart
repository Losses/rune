import 'dart:collection';

import '../bindings/bindings.dart';

class QueryList extends ListBase<(String, String)> {
  final List<(String, String)> _inner;

  const QueryList([List<(String, String)> elements = const []])
      : _inner = elements;

  @override
  int get length => _inner.length;

  @override
  set length(int newLength) {
    throw UnsupportedError("Cannot change the length of a const list");
  }

  @override
  (String, String) operator [](int index) => _inner[index];

  @override
  void operator []=(int index, (String, String) value) {
    throw UnsupportedError("Cannot modify a const list");
  }

  @override
  bool operator ==(Object other) {
    if (identical(this, other)) return true;
    if (other is! QueryList) return false;
    if (length != other.length) return false;
    for (int i = 0; i < length; i++) {
      if (this[i] != other[i]) return false;
    }
    return true;
  }

  @override
  int get hashCode {
    return Object.hashAll(_inner);
  }

  List<MixQuery> toQueryList() {
    return _inner
        .map((x) => MixQuery(operator: x.$1, parameter: x.$2))
        .toList();
  }

  static fromMixQuery(List<MixQuery> queries) {
    return QueryList(queries.map((x) => (x.operator, x.parameter)).toList());
  }

  static bool computeIsAlbumQuery(QueryList elements) {
    final libQueries = elements.where((x) => x.$1.startsWith('lib::'));
    return libQueries.length == 1 && libQueries.first.$1 == 'lib::album';
  }
}
