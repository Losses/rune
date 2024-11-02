import 'package:flutter/foundation.dart';

import '../utils/navigation/navigation_item.dart';

enum RouteRelation {
  parent,
  child,
  sameLevelAhead,
  sameLevelBehind,
  same,
  crossLevel,
}

class TransitionCalculationProvider with ChangeNotifier {
  final List<NavigationItem> navigationItems;
  final Map<String, NavigationItem> _pathToItemMap = {};
  final Map<String, String?> _pathToParentMap = {};

  TransitionCalculationProvider({required this.navigationItems}) {
    _initializeMaps();
  }

  void _initializeMaps() {
    void traverse(List<NavigationItem> items, String? parentPath) {
      for (var item in items) {
        if (!item.zuneOnly) {
          _pathToItemMap[item.path] = item;
          _pathToParentMap[item.path] = parentPath;
        }

        traverse(item.children ?? [], item.path);
      }
    }

    traverse(navigationItems, null);
  }

  // Compare route relationships
  RouteRelation compareRoute(String? from, String? to) {
    final fromItem = _pathToItemMap[from];
    final toItem = _pathToItemMap[to];

    if (fromItem == null || toItem == null) {
      return RouteRelation.crossLevel;
    }

    if (from == to) {
      return RouteRelation.same;
    }

    final currentParent = _pathToParentMap[from];
    final targetParent = _pathToParentMap[to];

    if (currentParent == to) {
      return RouteRelation.parent;
    }

    if (targetParent == from) {
      return RouteRelation.child;
    }

    if (currentParent == targetParent) {
      final siblings = _pathToItemMap[currentParent]?.children ?? [];
      final currentIndex = siblings.indexOf(fromItem);
      final targetIndex = siblings.indexOf(toItem);

      if (currentIndex < targetIndex) {
        return RouteRelation.sameLevelBehind;
      } else {
        return RouteRelation.sameLevelAhead;
      }
    }

    return RouteRelation.crossLevel;
  }
}
