import 'package:fluent_ui/fluent_ui.dart';

class RouterPathProvider with ChangeNotifier {
  String? path = '/library';
  Object? parameter;

  void update(String? x, Object? p) {
    if (x == path && parameter == p) return;
    path = x;
    parameter = p;

    notifyListeners();
  }
}

final $router = RouterPathProvider();
