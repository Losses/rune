import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import 'package:player/screens/settings_test/widgets/directory_section.dart';
import 'package:player/screens/settings_test/widgets/select_input_section.dart';
import 'package:player/screens/settings_test/widgets/slider_section.dart';
import 'package:player/widgets/playback_controller/playback_mode_button.dart';
import 'package:player/widgets/playback_controller/utils/playback_mode.dart';

import '../../../messages/album.pb.dart';
import '../../../messages/search.pb.dart';
import '../../../messages/media_file.pb.dart';
import '../../../messages/artist.pbserver.dart';
import '../../../messages/playlist.pbserver.dart';
import '../../../utils/chip_input/chip_input.dart';

class ChipInputSection extends StatelessWidget {
  final String title;
  final Future<List<AutoSuggestBoxItem<int>>> Function() getInitResult;
  final Future<List<AutoSuggestBoxItem<int>>> Function(String) searchForItems;

  const ChipInputSection({
    required this.title,
    required this.getInitResult,
    required this.searchForItems,
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(title),
        const SizedBox(height: 4),
        ChipInput(
          getInitResult: getInitResult,
          searchFor: searchForItems,
        ),
        const SizedBox(height: 12),
      ],
    );
  }
}

Future<Map<String, List<int>>> searchFor(String query, String field) async {
  final searchRequest =
      SearchForRequest(queryStr: query, fields: [field], n: 30);
  searchRequest.sendSignalToRust(); // GENERATED

  final message = (await SearchForResponse.rustSignalStream.first).message;

  final Map<String, List<int>> result = {};

  result['artists'] = message.artists;
  result['albums'] = message.albums;
  result['playlists'] = message.playlists;
  result['tracks'] = message.tracks;

  return result;
}

final modeSelectItems = [
  SelectItem(value: "99", title: "Default", icon: Symbols.change_history),
  ...[PlaybackMode.sequential, PlaybackMode.repeatAll, PlaybackMode.shuffle]
      .map(
    (x) => SelectItem(
      value: modeToInt(x).toString(),
      title: modeToLabel(x),
      icon: modeToIcon(x),
    ),
  ),
];

final recommendSelectItems = [
  SelectItem(
      value: "99", title: "No Recommendation", icon: Symbols.circles_ext),
  SelectItem(value: "98", title: "Based on All", icon: Symbols.blur_circular),
  SelectItem(value: "0", title: "Group 1", icon: Symbols.counter_1),
  SelectItem(value: "1", title: "Group 2", icon: Symbols.counter_2),
  SelectItem(value: "2", title: "Group 3", icon: Symbols.counter_3),
  SelectItem(value: "3", title: "Group 4", icon: Symbols.counter_4),
  SelectItem(value: "4", title: "Group 5", icon: Symbols.counter_5),
  SelectItem(value: "5", title: "Group 6", icon: Symbols.counter_6),
  SelectItem(value: "6", title: "Group 7", icon: Symbols.counter_7),
  SelectItem(value: "7", title: "Group 8", icon: Symbols.counter_8),
  SelectItem(value: "8", title: "Group 9", icon: Symbols.counter_9),
];

final sortSelectItems = [
  SelectItem(value: "default", title: "Default", icon: Symbols.stream),
  SelectItem(
      value: "last_modified", title: "Last Modified", icon: Symbols.refresh),
  SelectItem(
      value: "duration", title: "Duration", icon: Symbols.access_time_filled),
  SelectItem(
      value: "playedthrough",
      title: "Times Played Through",
      icon: Symbols.line_end_circle),
  SelectItem(value: "skipped", title: "Times Skipped", icon: Symbols.step_over),
];

class EditMixDialog extends StatefulWidget {
  const EditMixDialog({super.key});

  @override
  State<EditMixDialog> createState() => _EditMixDialogState();
}

class _EditMixDialogState extends State<EditMixDialog> {
  @override
  Widget build(BuildContext context) {
    final height = MediaQuery.of(context).size.height;
    const reduce = 400.0;

    return ContentDialog(
      title: const Column(
        children: [
          SizedBox(height: 8),
          Text("Create Mix"),
        ],
      ),
      content: Container(
          constraints: BoxConstraints(
            maxHeight: height < reduce ? reduce : height - reduce,
          ),
          child: SingleChildScrollView(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                ChipInputSection(
                  title: 'Artists',
                  getInitResult: () => _getInitResult(_fetchArtistSummary),
                  searchForItems: (query) =>
                      _searchItems(query, 'artists', _fetchArtistsByIds),
                ),
                ChipInputSection(
                  title: 'Albums',
                  getInitResult: () => _getInitResult(_fetchAlbumSummary),
                  searchForItems: (query) =>
                      _searchItems(query, 'albums', _fetchAlbumsByIds),
                ),
                ChipInputSection(
                  title: 'Playlists',
                  getInitResult: () => _getInitResult(_fetchPlaylistSummary),
                  searchForItems: (query) =>
                      _searchItems(query, 'playlists', _fetchPlaylistsByIds),
                ),
                ChipInputSection(
                  title: 'Tracks',
                  getInitResult: () => _getInitResult(_fetchTrackSummary),
                  searchForItems: (query) =>
                      _searchItems(query, 'tracks', _fetchTrackByIds),
                ),
                const DirectorySection(),
                const SliderSection(title: "Limit"),
                SelectInputSection(
                  title: "Mode",
                  items: modeSelectItems,
                  defaultValue: '99',
                ),
                SelectInputSection(
                  title: "Recommendation",
                  items: recommendSelectItems,
                  defaultValue: '99',
                ),
                SelectInputSection(
                  title: "Sort By",
                  items: sortSelectItems,
                  defaultValue: 'default',
                ),
                ToggleSwitch(
                  checked: false,
                  onChanged: (v) => print(v),
                  content: Expanded(
                    child: Text("Liked Only"),
                  ),
                  leadingContent: true,
                ),
              ],
            ),
          )),
      actions: [
        FilledButton(
          child: const Text('Query'),
          onPressed: () {
            Navigator.pop(context, 'User deleted file');
            // Delete file here
          },
        ),
        Button(
          child: const Text('Cancel'),
          onPressed: () => Navigator.pop(context, 'User canceled dialog'),
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
