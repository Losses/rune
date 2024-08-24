import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar.dart';

class SettingsPage extends StatefulWidget {
  const SettingsPage({super.key});

  @override
  State<SettingsPage> createState() => _SettingsPageState();
}

class _SettingsPageState extends State<SettingsPage> {
  @override
  Widget build(BuildContext context) {
    return ScaffoldPage(
      content: Column(children: [
        const NavigationBarPlaceholder(),
        const TextBox(),
        Center(
          child: Text(
            'Hello, World!',
            style: FluentTheme.of(context).typography.title,
          ),
        )
      ]),
    );
  }
}
