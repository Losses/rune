import 'package:fluent_ui/fluent_ui.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      header: const PageHeader(
        title: Text('Hello World with Fluent UI'),
      ),
      content: Center(
        child: Text(
          'Hello, World!',
          style: FluentTheme.of(context).typography.title,
        ),
      ),
    );
  }
}
