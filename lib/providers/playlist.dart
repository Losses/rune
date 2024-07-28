import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../messages/playback.pb.dart';

class PlaylistProvider with ChangeNotifier {
  List<PlaylistItem> _items = [];

  List<PlaylistItem> get items => _items;

  void updatePlaylist(List<PlaylistItem> newItems) {
    _items = newItems;
    notifyListeners();
  }
}

class PlaylistUpdateHandler {
  static void init(BuildContext context) {
    PlaylistUpdate.rustSignalStream.listen((event) {
      final playlistUpdate = event.message;
      final items = playlistUpdate.items;

      Provider.of<PlaylistProvider>(context, listen: false)
          .updatePlaylist(items);
    });
  }
}
