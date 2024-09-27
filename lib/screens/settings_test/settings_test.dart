import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/tile/fancy_cover.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';
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
    return const Column(children: [
      NavigationBarPlaceholder(),
      FancyCover(
        size: 256,
        texts: (
          "Default Title Name",
          "Artist Name",
          "Total Time 12:35",
        ),
        configIndex: 8,
      ),
      PlaybackPlaceholder()
    ]);
  }
}
