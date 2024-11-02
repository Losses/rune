import 'package:fluent_ui/fluent_ui.dart';

escapeFromSearch(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;

  if (path == '/search') {
    if (Navigator.canPop(context)) {
      Navigator.pop(context);
    } else {
      Navigator.pushNamed(context, '/library');
    }

    return true;
  }

  return false;
}
