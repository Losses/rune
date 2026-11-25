import 'package:fluent_ui/fluent_ui.dart';

import '../main.dart';

class RouterPathProvider with ChangeNotifier {
  String? path = cafeMode ? '/cover_wall' : '/library';
  Object? parameter;

  void update(String? x, Object? p) {
    if (x == path && parameter == p) return;
    path = x;
    parameter = p;

    notifyListeners();
  }
}

final $router = RouterPathProvider();
