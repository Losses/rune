import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/chip_input/chip_input.dart';
import '../../../widgets/directory/directory_tree.dart';
import '../../../screens/settings_test/utils/mix_editor_data.dart';
import '../../../screens/settings_test/utils/slider_controller.dart';
import '../../../screens/settings_test/utils/select_input_controller.dart';
import '../../../screens/settings_test/utils/toggle_switch_controller.dart';

class MixEditorController extends ChangeNotifier {
  final ChipInputController<int> artistsController = ChipInputController<int>();
  final ChipInputController<int> albumsController = ChipInputController<int>();
  final ChipInputController<int> playlistsController =
      ChipInputController<int>();
  final ChipInputController<int> tracksController = ChipInputController<int>();
  final DirectoryTreeController directoryController = DirectoryTreeController();
  final SliderController limitController = SliderController();
  final SelectInputController modeController = SelectInputController('99');
  final SelectInputController recommendationController =
      SelectInputController('');
  final SelectInputController sortByController =
      SelectInputController('default');
  final ToggleSwitchController likedController = ToggleSwitchController(false);

  MixEditorController() {
    _initListeners();
  }

  void _initListeners() {
    artistsController.addListener(_notifyListeners);
    albumsController.addListener(_notifyListeners);
    playlistsController.addListener(_notifyListeners);
    tracksController.addListener(_notifyListeners);
    directoryController.addListener(_notifyListeners);
    limitController.addListener(_notifyListeners);
    modeController.addListener(_notifyListeners);
    recommendationController.addListener(_notifyListeners);
    sortByController.addListener(_notifyListeners);
    likedController.addListener(_notifyListeners);
  }

  void _notifyListeners() {
    notifyListeners();
  }

  @override
  void dispose() {
    artistsController.removeListener(_notifyListeners);
    albumsController.removeListener(_notifyListeners);
    playlistsController.removeListener(_notifyListeners);
    tracksController.removeListener(_notifyListeners);
    directoryController.removeListener(_notifyListeners);
    limitController.removeListener(_notifyListeners);
    modeController.removeListener(_notifyListeners);
    recommendationController.removeListener(_notifyListeners);
    sortByController.removeListener(_notifyListeners);
    likedController.removeListener(_notifyListeners);

    artistsController.dispose();
    albumsController.dispose();
    playlistsController.dispose();
    tracksController.dispose();
    directoryController.dispose();
    limitController.dispose();
    modeController.dispose();
    recommendationController.dispose();
    sortByController.dispose();
    likedController.dispose();

    super.dispose();
  }

  MixEditorData getData() {
    return MixEditorData(
      artists: artistsController.selectedItems
          .map((item) => item.value)
          .where((value) => value != null)
          .cast<int>()
          .toList(),
      albums: albumsController.selectedItems
          .map((item) => item.value)
          .where((value) => value != null)
          .cast<int>()
          .toList(),
      playlists: playlistsController.selectedItems
          .map((item) => item.value)
          .where((value) => value != null)
          .cast<int>()
          .toList(),
      tracks: tracksController.selectedItems
          .map((item) => item.value)
          .where((value) => value != null)
          .cast<int>()
          .toList(),
      directories: directoryController.value ?? {},
      limit: limitController.value,
      mode: modeController.selectedValue ?? '99',
      recommendation: recommendationController.selectedValue ?? '99',
      sortBy: sortByController.selectedValue ?? 'default',
      likedOnly: likedController.isChecked,
    );
  }

  void setData(MixEditorData data) {
    artistsController.clearItems();
    for (var artist in data.artists) {
      artistsController.addItem(
          AutoSuggestBoxItem<int>(value: artist, label: artist.toString()));
    }

    albumsController.clearItems();
    for (var album in data.albums) {
      albumsController.addItem(
          AutoSuggestBoxItem<int>(value: album, label: album.toString()));
    }

    playlistsController.clearItems();
    for (var playlist in data.playlists) {
      playlistsController.addItem(
          AutoSuggestBoxItem<int>(value: playlist, label: playlist.toString()));
    }

    tracksController.clearItems();
    for (var track in data.tracks) {
      tracksController.addItem(
          AutoSuggestBoxItem<int>(value: track, label: track.toString()));
    }

    directoryController.value = data.directories;
    limitController.value = data.limit;
    modeController.selectedValue = data.mode;
    recommendationController.selectedValue = data.recommendation;
    sortByController.selectedValue = data.sortBy;
    likedController.isChecked = data.likedOnly;
  }
}
