import 'package:fluent_ui/fluent_ui.dart';

class RouterPathProvider with ChangeNotifier {
  String? path = '/library';

  void update(String? x) {
    if (x == path) return;
    path = x;

    notifyListeners();
  }
}

final $routerPath = RouterPathProvider();
