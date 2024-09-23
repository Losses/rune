import 'dart:collection';

class QueryList extends ListBase<(String, String)> {
  final List<(String, String)> _inner = [];

  QueryList([Iterable<(String, String)>? elements]) {
    if (elements != null) {
      _inner.addAll(elements);
    }
  }

  @override
  int get length => _inner.length;

  @override
  set length(int newLength) {
    _inner.length = newLength;
  }

  @override
  (String, String) operator [](int index) => _inner[index];

  @override
  void operator []=(int index, (String, String) value) {
    _inner[index] = value;
  }

  @override
  void add((String, String) element) {
    _inner.add(element);
  }

  @override
  void addAll(Iterable<(String, String)> iterable) {
    _inner.addAll(iterable);
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
}
