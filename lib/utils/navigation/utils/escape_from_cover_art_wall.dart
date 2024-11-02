import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

escapeFromCoverArtWall(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;
  if (path == '/cover_wall') {
    if (Navigator.canPop(context)) {
      Navigator.pop(context);
    } else {
      $push(context, '/library');
    }

    return true;
  }

  return false;
}
