import 'package:fluent_ui/fluent_ui.dart';

class ProgressButton extends StatelessWidget {
  final void Function()? onPressed;
  final double? progress;
  final bool? filled;

  const ProgressButton({
    super.key,
    required this.title,
    required this.onPressed,
    required this.progress,
    required this.filled,
  });

  final String title;

  @override
  Widget build(BuildContext context) {
    final child = Row(
      mainAxisAlignment: MainAxisAlignment.center,
      mainAxisSize: MainAxisSize.max,
      children: [
        SizedBox(
          width: 16,
          height: 16,
          child: OverflowBox(
            maxWidth: 16,
            maxHeight: 16,
            child: ProgressRing(
              strokeWidth: 2,
              value: progress != null ? progress! * 100 : null,
            ),
          ),
        ),
        const SizedBox(
          width: 8,
        ),
        Text(title)
      ],
    );

    if (filled == true) {
      FilledButton(
        onPressed: onPressed,
        child: child,
      );
    }

    return Button(
      onPressed: onPressed,
      child: child,
    );
  }
}
