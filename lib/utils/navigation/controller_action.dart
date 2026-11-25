import 'package:fluent_ui/fluent_ui.dart';

import '../../bindings/bindings.dart';
import '../../utils/navigation/controller_intent.dart';

class ControllerAction extends Action<ControllerIntent> {
  final BuildContext context;

  ControllerAction(this.context);

  // A control the current target hides or disables takes no shortcut either,
  // and its key goes on to whatever else takes it.
  @override
  bool isEnabled(covariant ControllerIntent intent) =>
      intent.entry.capability(context, false) is RequestCapabilityAvailable;

  @override
  void invoke(covariant ControllerIntent intent) {
    final fn = intent.entry.onShortcut;

    if (fn != null) {
      fn(context);
    }
  }
}
