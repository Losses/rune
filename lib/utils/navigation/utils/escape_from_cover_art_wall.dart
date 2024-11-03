import 'package:fluent_ui/fluent_ui.dart';

import '../../../providers/router_path.dart';
import '../../router/navigation.dart';

escapeFromCoverArtWall(BuildContext context) {
  final path = $routerPath.path;

  if (path == '/cover_wall') {
    if (!$pop()) {
      $push(context, '/library');
    }

    return true;
  }

  return false;
}
