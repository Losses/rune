import '../../../providers/router_path.dart';

import '../../router/navigation.dart';

escapeFromSearch() {
  final path = $routerPath.path;

  if (path == '/search') {
    if (!$pop()) {
      $replace('/library');
    }

    return true;
  }

  return false;
}
