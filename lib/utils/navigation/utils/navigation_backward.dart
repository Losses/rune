import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/config/navigation_query.dart';

bool navigationBackward(BuildContext context) {
  final canPop = Navigator.canPop(context);

  if (!canPop) {
    final path = ModalRoute.of(context)?.settings.name;
    final parent = navigationQuery.getParent(path, false);
    if (parent != null && parent.path != '/' && parent.path != '/home') {
      Navigator.pushReplacementNamed(context, parent.path);
    }
  }

  return !canPop;
}

navigateBackwardWithPop(BuildContext context) {
  final path = ModalRoute.of(context)?.settings.name;

  if (!navigationBackward(context)) {
    Navigator.pop(context);
  } else if (path != '/library') {
    Navigator.pushReplacementNamed(context, '/library');
  }
}
