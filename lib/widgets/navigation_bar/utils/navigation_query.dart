import 'dart:collection';

import '../../../widgets/navigation_bar/utils/navigation_item.dart';

class NavigationQuery {
  final HashMap<(String, bool), NavigationItem> _pathToItem = HashMap();
  final HashMap<(String, bool), (String, bool)> _pathToParent = HashMap();
  final HashMap<(String, bool), List<NavigationItem>> _pathToChildren =
      HashMap();
  final List<NavigationItem> _rootItems = [];

  NavigationQuery(List<NavigationItem> items) {
    for (var item in items) {
      _addItem(item, null);
    }
  }

  void _addItem(NavigationItem item, (String, bool)? parentPath) {
    final key = (item.path, item.zuneOnly);

    _pathToItem[key] = item;
    if (parentPath != null) {
      _pathToParent[key] = parentPath;
      if (!_pathToChildren.containsKey(parentPath)) {
        _pathToChildren[parentPath] = [];
      }
      _pathToChildren[parentPath]!.add(item);
    } else {
      _rootItems.add(item);
    }

    final children = item.children;
    if (children != null) {
      for (var child in children) {
        _addItem(child, (item.path, item.zuneOnly));
      }
    }
  }

  NavigationItem? getItem(String? path, bool zuneOnly) {
    return _pathToItem[(path, zuneOnly)] ?? _pathToItem[(path, false)];
  }

  NavigationItem? getParent(String? path, bool zuneOnly) {
    final parentPath = _pathToParent[(path, zuneOnly)];
    if (parentPath != null) {
      return _pathToItem[parentPath];
    }

    if (zuneOnly) {
      final parentPath = _pathToParent[(path, false)];
      if (parentPath != null) {
        return _pathToItem[parentPath];
      }
    }

    return null;
  }

  List<NavigationItem>? getChildren(String? path, bool zuneOnly) {
    return _pathToChildren[(path, zuneOnly)];
  }

  List<NavigationItem>? getSiblings(String? path, bool zuneOnly) {
    var parentPath = _pathToParent[(path, zuneOnly)] ?? _pathToParent[(path, false)];
    if (parentPath == null) {
      // If there's no parent, it means this item is a root item
      // Its siblings are all other root items
      return _rootItems.toList();
    }

    var siblings = _pathToChildren[parentPath];
    if (siblings == null) {
      return null;
    }

    // Filter out the item itself from the list of siblings
    return siblings.toList();
  }
}
