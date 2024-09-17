import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:player/messages/recommend.pbserver.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsMixPage extends StatefulWidget {
  const SettingsMixPage({super.key});

  @override
  State<SettingsMixPage> createState() => _SettingsMixPageState();
}

class _SettingsMixPageState extends State<SettingsMixPage> {
  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Padding(
        padding: const EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            
          ],
        ),
      )
    ]);
  }
}
