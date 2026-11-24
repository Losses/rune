import 'package:fluent_ui/fluent_ui.dart';

class BeltContainer extends StatelessWidget {
  const BeltContainer({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(
        bottom: 12,
      ),
      child: Align(
        alignment: Alignment.centerLeft,
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 100),
          child: child,
        ),
      ),
    );
  }
}
