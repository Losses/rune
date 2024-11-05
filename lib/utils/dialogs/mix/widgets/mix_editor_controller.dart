import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/dialogs/mix/widgets/number_section.dart';
import '../../../../widgets/directory/directory_tree.dart';

import '../../../chip_input/chip_input.dart';

import '../utils/mix_editor_data.dart';
import '../utils/slider_controller.dart';
import '../utils/select_input_controller.dart';
import '../utils/toggle_switch_controller.dart';

class MixEditorController extends ChangeNotifier {
  final TextEditingController titleController = TextEditingController();
  final TextEditingController groupController = TextEditingController();
  final ChipInputController<int> artistsController = ChipInputController<int>();
  final ChipInputController<int> albumsController = ChipInputController<int>();
  final ChipInputController<int> playlistsController =
      ChipInputController<int>();
  final ChipInputController<int> tracksController = ChipInputController<int>();
  final TextEditingController randomTrackController = TextEditingController();
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
    titleController.addListener(_notifyListeners);
    groupController.addListener(_notifyListeners);
    artistsController.addListener(_notifyListeners);
    albumsController.addListener(_notifyListeners);
    playlistsController.addListener(_notifyListeners);
    tracksController.addListener(_notifyListeners);
    randomTrackController.addListener(_notifyListeners);
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
    titleController.removeListener(_notifyListeners);
    groupController.removeListener(_notifyListeners);
    artistsController.removeListener(_notifyListeners);
    albumsController.removeListener(_notifyListeners);
    playlistsController.removeListener(_notifyListeners);
    tracksController.removeListener(_notifyListeners);
    randomTrackController.removeListener(_notifyListeners);
    directoryController.removeListener(_notifyListeners);
    limitController.removeListener(_notifyListeners);
    modeController.removeListener(_notifyListeners);
    recommendationController.removeListener(_notifyListeners);
    sortByController.removeListener(_notifyListeners);
    likedController.removeListener(_notifyListeners);

    titleController.dispose();
    groupController.dispose();
    artistsController.dispose();
    albumsController.dispose();
    playlistsController.dispose();
    tracksController.dispose();
    randomTrackController.dispose();
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
      title: titleController.value.text,
      group: groupController.value.text,
      artists: artistsController.selectedItems
          .map((item) => (item.value, item.label))
          .where((value) => value.$1 != null)
          .cast<(int, String)>()
          .toList(),
      albums: albumsController.selectedItems
          .map((item) => (item.value, item.label))
          .where((value) => value.$1 != null)
          .cast<(int, String)>()
          .toList(),
      playlists: playlistsController.selectedItems
          .map((item) => (item.value, item.label))
          .where((value) => value.$1 != null)
          .cast<(int, String)>()
          .toList(),
      tracks: tracksController.selectedItems
          .map((item) => (item.value, item.label))
          .where((value) => value.$1 != null)
          .cast<(int, String)>()
          .toList(),
      randomTracks: bestInt(randomTrackController.value.text),
      directories: directoryController.value ?? {},
      limit: limitController.value,
      mode: modeController.selectedValue ?? '99',
      recommendation: recommendationController.selectedValue ?? '99',
      sortBy: sortByController.selectedValue ?? 'default',
      likedOnly: likedController.isChecked,
    );
  }

  void setData(MixEditorData data) {
    titleController.text = data.title;
    groupController.text = data.group;

    artistsController.clearItems();
    for (var artist in data.artists) {
      artistsController
          .addItem(AutoSuggestBoxItem<int>(value: artist.$1, label: artist.$2));
    }

    albumsController.clearItems();
    for (var album in data.albums) {
      albumsController
          .addItem(AutoSuggestBoxItem<int>(value: album.$1, label: album.$2));
    }

    playlistsController.clearItems();
    for (var playlist in data.playlists) {
      playlistsController.addItem(
          AutoSuggestBoxItem<int>(value: playlist.$1, label: playlist.$2));
    }

    tracksController.clearItems();
    for (var track in data.tracks) {
      tracksController
          .addItem(AutoSuggestBoxItem<int>(value: track.$1, label: track.$2));
    }

    directoryController.value = data.directories;
    limitController.value = data.limit;
    modeController.selectedValue = data.mode;
    recommendationController.selectedValue = data.recommendation;
    sortByController.selectedValue = data.sortBy;
    likedController.isChecked = data.likedOnly;
  }
}
