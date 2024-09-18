import 'dart:collection';

import '../../../widgets/navigation_bar/utils/navigation_item.dart';

class NavigationQuery {
  final HashMap<String, NavigationItem> _pathToItem = HashMap();
  final HashMap<String, String> _pathToParent = HashMap();
  final HashMap<String, List<NavigationItem>> _pathToChildren = HashMap();
  final List<NavigationItem> _rootItems = [];

  NavigationQuery(List<NavigationItem> items) {
    for (var item in items) {
      _addItem(item, null);
    }
  }

  void _addItem(NavigationItem item, String? parentPath) {
    _pathToItem[item.path] = item;
    if (parentPath != null) {
      _pathToParent[item.path] = parentPath;
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
        _addItem(child, item.path);
      }
    }
  }

  NavigationItem? getItem(String? path) {
    return _pathToItem[path];
  }

  NavigationItem? getParent(String? path) {
    var parentPath = _pathToParent[path];
    if (parentPath != null) {
      return _pathToItem[parentPath];
    }
    return null;
  }

  List<NavigationItem>? getChildren(String? path) {
    return _pathToChildren[path];
  }

  List<NavigationItem>? getSiblings(String? path) {
    var parentPath = _pathToParent[path];
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
