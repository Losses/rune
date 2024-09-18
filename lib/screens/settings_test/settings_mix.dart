
import 'package:fluent_ui/fluent_ui.dart';

import '../../screens/settings_test/widgets/edit_mix_dialog.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsMixPage extends StatefulWidget {
  const SettingsMixPage({super.key});

  @override
  State<SettingsMixPage> createState() => _SettingsMixPageState();
}

class _SettingsMixPageState extends State<SettingsMixPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Padding(
        padding: EdgeInsets.symmetric(vertical: 24, horizontal: 32),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            EditMixDialog(),
          ],
        ),
      )
    ]);
  }
}
