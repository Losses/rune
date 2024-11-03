import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/router_path.dart';

import '../../router/navigation.dart';

escapeFromSearch(BuildContext context) {
  final path = $routerPath.path;

  if (path == '/search') {
    if (!$pop()) {
      $replace(context, '/library');
    }

    return true;
  }

  return false;
}
