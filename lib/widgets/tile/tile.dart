import 'package:fluent_ui/fluent_ui.dart';

class Tile extends StatelessWidget {
  const Tile({
    super.key,
    required this.onPressed,
    required this.child,
  });

  final VoidCallback? onPressed;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Button(
      style: const ButtonStyle(
        padding: WidgetStatePropertyAll(
          EdgeInsets.all(0),
        ),
      ),
      onPressed: onPressed,
      child: ClipRRect(
        borderRadius: BorderRadius.circular(3),
        child: SizedBox.expand(
          child: child,
        ),
      ),
    );
  }
}
