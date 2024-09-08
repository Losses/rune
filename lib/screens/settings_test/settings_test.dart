import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Padding(
        padding: EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text("Settings Home"),
          ],
        ),
      )
    ]);
  }
}
