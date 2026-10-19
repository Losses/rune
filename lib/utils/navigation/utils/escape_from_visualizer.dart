import '../../../providers/router_path.dart';

import '../../router/navigation.dart';

escapeFromVisualizer() {
  final path = $router.path;

  if (path == '/visualizer') {
    if (!$pop()) {
      $replace('/library');
    }

    return true;
  }

  return false;
}
