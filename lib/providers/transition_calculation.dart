import 'package:flutter/foundation.dart';

import '../../widgets/navigation_bar/utils/navigation_item.dart';

enum RouteRelation {
  parent,
  child,
  sameLevelAhead,
  sameLevelBehind,
  same,
  crossLevel,
}

class TransitionCalculationProvider with ChangeNotifier {
  String _currentRoute = '/library';

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

  // Register the current route
  void registerRoute(String path) {
    _currentRoute = path;
  }

  // Get the current route
  String get currentRoute => _currentRoute;

  // Compare route relationships
  RouteRelation compareRoute(String path) {
    final currentItem = _pathToItemMap[_currentRoute];
    final targetItem = _pathToItemMap[path];

    if (currentItem == null || targetItem == null) {
      return RouteRelation.crossLevel;
    }

    if (_currentRoute == path) {
      return RouteRelation.same;
    }

    final currentParent = _pathToParentMap[_currentRoute];
    final targetParent = _pathToParentMap[path];

    if (currentParent == path) {
      return RouteRelation.parent;
    }

    if (targetParent == _currentRoute) {
      return RouteRelation.child;
    }

    if (currentParent == targetParent) {
      final siblings = _pathToItemMap[currentParent]?.children ?? [];
      final currentIndex = siblings.indexOf(currentItem);
      final targetIndex = siblings.indexOf(targetItem);

      if (currentIndex < targetIndex) {
        return RouteRelation.sameLevelBehind;
      } else {
        return RouteRelation.sameLevelAhead;
      }
    }

    return RouteRelation.crossLevel;
  }
}
