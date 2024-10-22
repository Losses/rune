import 'package:fluent_ui/fluent_ui.dart';

class SettingsBlockTitle extends StatelessWidget {
  const SettingsBlockTitle({
    super.key,
    required this.title,
    required this.subtitle,
  });

  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);

    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisAlignment: MainAxisAlignment.start,
      children: [
        Text(
          title,
          style: theme.typography.body?.apply(fontSizeFactor: 1.1),
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.start,
        ),
        const SizedBox(
          height: 2,
        ),
        Text(
          subtitle,
          style: theme.typography.caption?.apply(
            color: theme.inactiveColor.withAlpha(160),
          ),
          overflow: TextOverflow.ellipsis,
          textAlign: TextAlign.start,
        ),
      ],
    );
  }
}
