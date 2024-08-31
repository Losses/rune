import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../messages/playback.pb.dart';

void openCollectionItemContextMenu(
    Offset localPosition,
    BuildContext context,
    GlobalKey contextAttachKey,
    FlyoutController contextController,
    String type,
    int id) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );

  contextController.showFlyout(
    position: position,
    builder: (context) => buildCollectionItemContextMenu(context, type, id),
  );
}

Widget buildCollectionItemContextMenu(
    BuildContext context, String type, int id) {
  return MenuFlyout(
    items: [
      MenuFlyoutItem(
        leading: const Icon(Symbols.play_circle),
        text: const Text('Start Playing'),
        onPressed: () => {
          StartPlayingCollectionRequest(type: type, id: id).sendSignalToRust()
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.playlist_add),
        text: const Text('Add to Queue'),
        onPressed: () => {
          AddToQueueCollectionRequest(type: type, id: id).sendSignalToRust()
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () => {
          StartRoamingCollectionRequest(type: type, id: id).sendSignalToRust()
        },
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.list_alt),
        text: const Text('Add to Playlist'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Auto Playlist'),
            onPressed: () async {
              Flyout.of(context).close();
            },
          ),
        ],
      ),
    ],
  );
}
