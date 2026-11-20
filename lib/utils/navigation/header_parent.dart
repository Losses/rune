import '../../config/navigation_query.dart';
import '../router/navigation_history.dart';
import 'navigation_item.dart';

/// The page the title above [path] names and goes to: search for a page opened
/// from it, so that going back keeps what was found, and else the page's own
/// parent.
NavigationItem? headerParent(
  String? path,
  NavigationHistory history,
  bool isZune,
) {
  const search = '/search';
  if (path != search && history.previousIs(search)) {
    return navigationQuery.getItem(search, isZune);
  }
  return navigationQuery.getParent(path, isZune);
}
