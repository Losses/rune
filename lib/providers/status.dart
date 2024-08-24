import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../messages/playback.pb.dart';

class PlaybackStatusProvider with ChangeNotifier {
  PlaybackStatus? _playbackStatus;

  PlaybackStatus? get playbackStatus => _playbackStatus;

  void updatePlaybackStatus(PlaybackStatus newStatus) {
    _playbackStatus = newStatus;
    notifyListeners();
  }
}

class PlaybackStatusUpdateHandler {
  static void init(BuildContext context) {
    PlaybackStatus.rustSignalStream.listen((event) {
      final playbackStatusUpdate = event.message;

      if (!context.mounted) return;
      Provider.of<PlaybackStatusProvider>(context, listen: false)
          .updatePlaybackStatus(playbackStatusUpdate);
    });
  }
}
