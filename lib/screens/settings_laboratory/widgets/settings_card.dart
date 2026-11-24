import '../../../widgets/settings/settings_container.dart';
import 'package:fluent_ui/fluent_ui.dart';

class SettingsCard extends StatelessWidget {
  final String title;
  final String description;
  final Widget content;

  const SettingsCard({
    super.key,
    required this.title,
    required this.description,
    required this.content,
  });

  @override
  Widget build(BuildContext context) {
    final typography = FluentTheme.of(context).typography;

    return SettingsContainer(
      margin: EdgeInsets.all(0),
      padding: EdgeInsets.all(8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(title, style: typography.subtitle),
          SizedBox(height: 8),
          Text(
            description,
            style: TextStyle(height: 1.4),
          ),
          SizedBox(height: 16),
          content,
        ],
      ),
    );
  }
}
