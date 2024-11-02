import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/router/router_transition_parameter.dart';

const initialRoute = '/library';

class RouterPath with ChangeNotifier {
  String path = initialRoute;
  RouterTransitionParameter? transition;

  RouterPath();

  void update(RouterTransitionParameter x) {
    path = x.to;
    transition = x;

    notifyListeners();
  }
}

final $routerPath = RouterPath();
