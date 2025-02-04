import 'package:fluent_ui/fluent_ui.dart';

class SubtitleButton extends StatelessWidget {
  const SubtitleButton({
    super.key,
    required this.onPressed,
    required this.title,
    required this.subtitle,
    this.selected = false,
  });

  final void Function() onPressed;
  final String title;
  final String subtitle;
  final bool selected;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return selected
        ? FilledButton(
            onPressed: onPressed,
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    title,
                    style: theme.typography.body?.apply(
                      fontSizeFactor: 1.1,
                      color: theme.inactiveBackgroundColor,
                    ),
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.start,
                  ),
                  const SizedBox(height: 4),
                  Text(
                    subtitle,
                    style: theme.typography.caption?.apply(
                      color: theme.inactiveBackgroundColor.withAlpha(160),
                    ),
                    textAlign: TextAlign.start,
                  ),
                ],
              ),
            ),
          )
        : Button(
            onPressed: onPressed,
            child: Padding(
              padding: const EdgeInsets.symmetric(vertical: 4),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.start,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    title,
                    style: theme.typography.body?.apply(fontSizeFactor: 1.1),
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.start,
                  ),
                  const SizedBox(height: 4),
                  Text(
                    subtitle,
                    style: theme.typography.caption?.apply(
                      color: theme.inactiveColor.withAlpha(160),
                    ),
                    textAlign: TextAlign.start,
                  ),
                ],
              ),
            ),
          );
  }
}
