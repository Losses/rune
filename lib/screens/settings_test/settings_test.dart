import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/playback_controller.dart';
import '../../widgets/directory/directory_tree.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

class _SettingsTestPageState extends State<SettingsTestPage> {
  Future<void> onSelectionChanged(Iterable<String> selectedItems) async {
    debugPrint('${selectedItems.map((i) => i)}');
  }

  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      Expanded(child: DirectoryTree(onSelectionChanged: onSelectionChanged)),
      const PlaybackPlaceholder()
    ]);
  }
}
