import 'package:fluent_ui/fluent_ui.dart';

class ProgressButton extends StatelessWidget {
  final Widget Function()? onPressed;

  const ProgressButton({
    super.key,
    required this.title,
    required this.onPressed,
  });

  final String title;

  @override
  Widget build(BuildContext context) {
    return Button(
        onPressed: onPressed,
        child: Row(
          children: [
            const SizedBox(
              width: 16,
              height: 16,
              child: OverflowBox(
                  maxWidth: 16,
                  maxHeight: 16,
                  child: ProgressRing(
                    strokeWidth: 2,
                  )),
            ),
            const SizedBox(
              width: 8,
            ),
            Text(title)
          ],
        ));
  }
}