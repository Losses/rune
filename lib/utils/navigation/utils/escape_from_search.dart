import 'package:fluent_ui/fluent_ui.dart';

import '../../router/navigation.dart';

escapeFromSearch(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;

  if (path == '/search') {
    if (Navigator.canPop(context)) {
      Navigator.pop(context);
    } else {
      $push(context, '/library');
    }

    return true;
  }

  return false;
}
