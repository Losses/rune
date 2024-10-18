import 'package:fluent_ui/fluent_ui.dart';

class ActivateLinkAction extends Action<ActivateIntent> {
  final BuildContext context;
  final VoidCallback? onTap;

  ActivateLinkAction(this.context, this.onTap);

  @override
  void invoke(covariant ActivateIntent intent) {
    if (onTap != null) {
      onTap!();
    }
  }
}
