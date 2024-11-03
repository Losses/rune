import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/router_path.dart';
import '../../../utils/router/navigation.dart';
import '../../../config/navigation_query.dart';

bool navigationBackward(BuildContext context) {
  final canPop = $canPop();

  if (!canPop) {
    final path = $routerPath.path;
    final parent = navigationQuery.getParent(path, false);
    if (parent != null && parent.path != '/' && parent.path != '/home') {
      $replace(context, parent.path);
    }
  }

  return !canPop;
}

navigateBackwardWithPop(BuildContext context) {
  final path = $routerPath.path;

  if (!navigationBackward(context)) {
    $pop();
  } else if (path != '/library') {
    $replace(context, '/library');
  }
}
