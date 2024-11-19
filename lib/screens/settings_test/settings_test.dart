import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/cover_wall_background/cover_wall_background.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return CoverWallBackground(
      seed: 114514,
      gap: 2,
    );
  }
}
