import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/navigation/controller_intent.dart';

class ControllerAction extends Action<ControllerIntent> {
  final BuildContext context;

  ControllerAction(this.context);

  @override
  void invoke(covariant ControllerIntent intent) {
    intent.entry.onShortcut(context);
  }
}
