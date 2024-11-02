import 'package:fluent_ui/fluent_ui.dart';

class GoRouterModalBarrierFix extends StatelessWidget {
  const GoRouterModalBarrierFix(
    this.child, {
    super.key,
  });

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Stack(
      children: [
        ModalBarrier(
          dismissible: true,
          color: Colors.transparent,
          onDismiss: () {},
        ),
        child,
      ],
    );
  }
}
