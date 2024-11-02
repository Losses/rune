import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/router/navigation.dart';
import '../../../config/navigation_query.dart';

bool navigationBackward(BuildContext context) {
  final canPop = $canPop();

  if (!canPop) {
    final path = ModalRoute.of(context)?.settings.name;
    final parent = navigationQuery.getParent(path, false);
    if (parent != null && parent.path != '/' && parent.path != '/home') {
      $replace(context, parent.path);
    }
  }

  return !canPop;
}

navigateBackwardWithPop(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;

  if (!navigationBackward(context)) {
    $pop();
  } else if (path != '/library') {
    $replace(context, '/library');
  }
}
