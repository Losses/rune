import '../../../providers/router_path.dart';

import '../../router/navigation.dart';

escapeFromLyrics() {
  final path = $router.path;

  if (path == '/lyrics') {
    if (!$pop()) {
      $replace('/library');
    }

    return true;
  }

  return false;
}
