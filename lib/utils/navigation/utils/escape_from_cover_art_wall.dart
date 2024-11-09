import '../../../providers/router_path.dart';

import '../../router/navigation.dart';

escapeFromCoverArtWall() {
  final path = $router.path;

  if (path == '/cover_wall') {
    if (!$pop()) {
      $replace('/library');
    }

    return true;
  }

  return false;
}
