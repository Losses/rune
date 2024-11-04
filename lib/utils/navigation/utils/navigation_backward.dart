import '../../../providers/router_path.dart';
import '../../../utils/router/navigation.dart';
import '../../../config/navigation_query.dart';

bool navigationBackward() {
  final canPop = $pop();

  if (!canPop) {
    final path = $router.path;
    final parent = navigationQuery.getParent(path, false);
    if (parent != null && parent.path != '/' && parent.path != '/home') {
      $replace(parent.path);
    }
  }

  return canPop;
}

navigateBackwardWithPop() {
  if (!navigationBackward()) {
    final path = $router.path;
    if (path != '/library') {
      $replace('/library');
    }
  }
}
