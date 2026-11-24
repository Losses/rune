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
        // Hit empty margins before the Navigator's modal barrier. In RuneStack
        // the playback controls share this space and handle the tap gestures.
        const AbsorbPointer(child: SizedBox.expand()),
        child,
      ],
    );
  }
}
