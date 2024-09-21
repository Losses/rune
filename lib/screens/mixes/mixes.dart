import 'package:fluent_ui/fluent_ui.dart';

import '../../../widgets/playback_controller/playback_placeholder.dart';
import '../../../widgets/navigation_bar/navigation_bar_placeholder.dart';

import 'mixes_list.dart';

class MixesPage extends StatefulWidget {
  const MixesPage({super.key});

  @override
  State<MixesPage> createState() => _MixesPageState();
}

class _MixesPageState extends State<MixesPage> {
  @override
  Widget build(BuildContext context) {
    return const Column(children: [
      NavigationBarPlaceholder(),
      Expanded(child: MixesListView()),
      PlaybackPlaceholder()
    ]);
  }
}
