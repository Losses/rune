import 'package:fluent_ui/fluent_ui.dart';

class RouterModalBarrierFix extends StatelessWidget {
  const RouterModalBarrierFix(
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
