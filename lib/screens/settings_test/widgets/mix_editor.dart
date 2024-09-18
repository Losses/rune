import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/screens/settings_test/utils/toggle_switch_controller.dart';
import 'package:player/screens/settings_test/widgets/toggle_switch_section.dart';

import '../../../utils/chip_input/chip_input.dart';

import '../../../screens/settings_test/utils/slider_controller.dart';
import '../../../screens/settings_test/utils/select_input_controller.dart';
import '../../../screens/settings_test/widgets/search_chip_input_section.dart';

import '../../../widgets/directory/directory_tree.dart';

import '../../../messages/album.pb.dart';
import '../../../messages/media_file.pb.dart';
import '../../../messages/artist.pbserver.dart';
import '../../../messages/playlist.pbserver.dart';

import '../config/mode_select_items.dart';
import '../config/sort_select_items.dart';
import '../config/recommend_select_items.dart';

import './slider_section.dart';
import './edit_mix_dialog.dart';
import './directory_section.dart';
import './select_input_section.dart';

class MixEditor extends StatefulWidget {
  const MixEditor({
    super.key,
  });

  @override
  State<MixEditor> createState() => _MixEditorState();
}

class _MixEditorState extends State<MixEditor> {
  final artistsController = ChipInputController<int>();
  final albumsController = ChipInputController<int>();
  final playlistsController = ChipInputController<int>();
  final tracksController = ChipInputController<int>();
  final directoryController = DirectoryTreeController();
  final limitController = SliderController();
  final modeController = SelectInputController('99');
  final recommendationController = SelectInputController('99');
  final sortByController = SelectInputController('default');
  final likedController = ToggleSwitchController(false);

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SearchChipInputSection(
          controller: artistsController,
          title: 'Artists',
          getInitResult: () => _getInitResult(_fetchArtistSummary),
          searchForItems: (query) =>
              _searchItems(query, 'artists', _fetchArtistsByIds),
        ),
        SearchChipInputSection(
          controller: albumsController,
          title: 'Albums',
          getInitResult: () => _getInitResult(_fetchAlbumSummary),
          searchForItems: (query) =>
              _searchItems(query, 'albums', _fetchAlbumsByIds),
        ),
        SearchChipInputSection(
          controller: playlistsController,
          title: 'Playlists',
          getInitResult: () => _getInitResult(_fetchPlaylistSummary),
          searchForItems: (query) =>
              _searchItems(query, 'playlists', _fetchPlaylistsByIds),
        ),
        SearchChipInputSection(
          controller: tracksController,
          title: 'Tracks',
          getInitResult: () => _getInitResult(_fetchTrackSummary),
          searchForItems: (query) =>
              _searchItems(query, 'tracks', _fetchTrackByIds),
        ),
        DirectorySection(controller: directoryController),
        SliderSection(controller: limitController, title: "Limit"),
        SelectInputSection(
          controller: modeController,
          title: "Mode",
          items: modeSelectItems,
          defaultValue: '99',
        ),
        SelectInputSection(
          controller: recommendationController,
          title: "Recommendation",
          items: recommendSelectItems,
          defaultValue: '99',
        ),
        SelectInputSection(
          controller: sortByController,
          title: "Sort By",
          items: sortSelectItems,
          defaultValue: 'default',
        ),
        ToggleSwitchSection(
          controller: likedController,
          content: const Text("Liked Only"),
        ),
      ],
    );
  }
}

Future<List<AutoSuggestBoxItem<int>>> _getInitResult(
    Future<List<(int, String)>> Function() fetchSummary) async {
  final summary = await fetchSummary();
  return summary
      .map((x) => AutoSuggestBoxItem<int>(value: x.$1, label: x.$2))
      .toList();
}

Future<List<AutoSuggestBoxItem<int>>> _searchItems<T>(
    String query,
    String field,
    Future<List<(int, String)>> Function(List<int>) fetchByIds) async {
  final ids = (await searchFor(query, field))[field];

  if (ids == null) return [];

  final items = await fetchByIds(ids);
  return items
      .map((x) => AutoSuggestBoxItem<int>(value: x.$1, label: x.$2))
      .toList();
}

Future<List<(int, String)>> _fetchArtistSummary() async {
  SearchArtistSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchArtistSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchAlbumSummary() async {
  SearchAlbumSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchAlbumSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchPlaylistSummary() async {
  SearchPlaylistSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchPlaylistSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchTrackSummary() async {
  SearchMediaFileSummaryRequest(n: 50).sendSignalToRust();
  return (await SearchMediaFileSummaryResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchArtistsByIds(List<int> ids) async {
  FetchArtistsByIdsRequest(ids: ids).sendSignalToRust();
  return (await FetchArtistsByIdsResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchAlbumsByIds(List<int> ids) async {
  FetchAlbumsByIdsRequest(ids: ids).sendSignalToRust();
  return (await FetchAlbumsByIdsResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchPlaylistsByIds(List<int> ids) async {
  FetchPlaylistsByIdsRequest(ids: ids).sendSignalToRust();
  return (await FetchPlaylistsByIdsResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.name))
      .toList();
}

Future<List<(int, String)>> _fetchTrackByIds(List<int> ids) async {
  FetchMediaFileByIdsRequest(ids: ids).sendSignalToRust();
  return (await FetchMediaFileByIdsResponse.rustSignalStream.first)
      .message
      .result
      .map((x) => (x.id, x.title))
      .toList();
}
