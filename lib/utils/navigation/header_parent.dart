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
  if (path != search && history.previousPageIs(search)) {
    return navigationQuery.getItem(search, isZune);
  }
  return navigationQuery.getParent(path, isZune);
}

enum HeaderTapAction { none, pop, replace }

/// What a tap on the parent title [target] does above the page [currentPath].
/// A tap on the page already shown does nothing, so that a second tap before
/// the header rebuilds cannot replace the page it came from.
HeaderTapAction headerTapAction(
  String target,
  String? currentPath,
  NavigationHistory history,
) {
  if (target == currentPath) return HeaderTapAction.none;
  if (history.previousIs(target)) return HeaderTapAction.pop;
  return HeaderTapAction.replace;
}
