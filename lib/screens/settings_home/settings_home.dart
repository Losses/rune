import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsHomePage extends StatefulWidget {
  const SettingsHomePage({super.key});

  @override
  State<SettingsHomePage> createState() => _SettingsHomePageState();
}

class _SettingsHomePageState extends State<SettingsHomePage> {
  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Button(
                child: const Text("Library"),
                onPressed: () => context.push('/settings/library')),
            Button(
                child: const Text("About"),
                onPressed: () => context.push('/settings/about')),
            Button(
                child: const Text("Mix"),
                onPressed: () => context.push('/settings/mix')),
          ],
        ),
      )
    ]);
  }
}
