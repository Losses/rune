import 'package:fluent_ui/fluent_ui.dart';

escapeFromCoverArtWall(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;
  if (path == '/cover_wall') {
    if (Navigator.canPop(context)) {
      Navigator.pop(context);
    } else {
      Navigator.pushNamed(context, '/library');
    }

    return true;
  }

  return false;
}
