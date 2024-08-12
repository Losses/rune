import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

class NavigationIndicatorHelper {
  final List<NavigationPaneItem> originalItems;
  final List<NavigationPaneItem> footerItems;
  final List<GoRoute> routes;
  final Map<String, int> _pathIndexMap = {};

  NavigationIndicatorHelper(this.originalItems, this.footerItems, this.routes) {
    _initializePathIndexMap();
  }

  int _matchNavigation(GoRoute route) {
    // Find the longest matching path in the current configuration
    String? matchedKey;
    int matchIndex = 0;
    int index = 0;

    // Function to update the matched key and index
    void updateMatch(NavigationPaneItem item) {
      final key = (item.key as ValueKey<String>).value;
      if (route.path.startsWith(key)) {
        if (matchedKey == null || key.length > matchedKey!.length) {
          matchedKey = key;
          matchIndex = index;
        }
      }
      index += 1;
    }

    // Populate map with originalItems
    for (NavigationPaneItem item in originalItems) {
      updateMatch(item);
    }

    // Populate map with footerItems
    for (NavigationPaneItem item in footerItems) {
      updateMatch(item);
    }

    return matchIndex;
  }

  void _initializePathIndexMap() {
    // Populate map with originalItems
    for (int i = 0; i < routes.length; i++) {
      final route = routes[i];
      _pathIndexMap[route.path] = _matchNavigation(route);
    }
  }

  int calculateSelectedIndex(BuildContext context) {
    final currentRoute = GoRouterState.of(context).fullPath;

    return _pathIndexMap[currentRoute] ?? 0;
  }
}
