import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/playback_controller/playlist.dart';

class PlaylistButton extends StatelessWidget {
  PlaylistButton({super.key});

  final contextController = FlyoutController();

  openContextMenu(BuildContext context) {
    contextController.showFlyout(
      autoModeConfiguration: FlyoutAutoConfiguration(
        preferredMode: FlyoutPlacementMode.topCenter,
      ),
      builder: (context) {
        return const Playlist();
      },
    );
  }

  @override
  Widget build(BuildContext context) {
    return FlyoutTarget(
      controller: contextController,
      child: IconButton(
        onPressed: () {
          openContextMenu(context);
        },
        icon: const Icon(Symbols.list_alt),
      ),
    );
  }
}
