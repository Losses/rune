import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../widgets/playback_controller/playback_placeholder.dart';

import 'settings_home_list.dart';

class SettingsHomePage extends StatefulWidget {
  const SettingsHomePage({super.key});

  @override
  State<SettingsHomePage> createState() => _SettingsHomePageState();
}

class _SettingsHomePageState extends State<SettingsHomePage> {
  final _layoutManager = StartScreenLayoutManager();

  @override
  void dispose() {
    super.dispose();
    _layoutManager.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return ChangeNotifierProvider<StartScreenLayoutManager>.value(
      value: _layoutManager,
      child: Column(
        children: [
          const NavigationBarPlaceholder(),
          Expanded(
            child: SettingsHomeList(
              layoutManager: _layoutManager,
            ),
          ),
          const PlaybackPlaceholder()
        ],
      ),
    );
  }
}
