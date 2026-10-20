import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../providers/router_path.dart';
import '../../providers/full_screen.dart';

import '../router/navigation.dart';

import 'utils/escape_from_search.dart';
import 'utils/escape_from_lyrics.dart';
import 'utils/escape_from_cover_art_wall.dart';

import 'escape_intent.dart';

const _escapablePaths = {'/search', '/lyrics', '/cover_wall'};

class EscapeAction extends Action<EscapeIntent> {
  EscapeAction();

  FullScreenProvider _fullScreen() =>
      Provider.of<FullScreenProvider>($context(), listen: false);

  /// A dialog or a flyout has the focus: leave Esc to it. While this action is
  /// disabled the key goes on to the app's default Esc → [DismissIntent].
  bool _popupOpen() {
    final focused = FocusManager.instance.primaryFocus?.context;
    final route = focused == null ? null : ModalRoute.of(focused);
    return route != null && (!route.opaque || !route.isCurrent);
  }

  @override
  bool isEnabled(EscapeIntent intent) {
    if (_popupOpen()) return false;

    return _fullScreen().isFullScreen ||
        _escapablePaths.contains($router.path);
  }

  @override
  void invoke(covariant EscapeIntent intent) {
    final fullScreen = _fullScreen();
    if (fullScreen.isFullScreen) {
      fullScreen.setFullScreen(false);
      return;
    }

    if (escapeFromSearch()) return;
    if (escapeFromLyrics()) return;
    if (escapeFromCoverArtWall()) return;
  }
}
