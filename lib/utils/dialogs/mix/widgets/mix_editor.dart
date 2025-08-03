import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';

import '../../../../utils/api/fetch_collection_by_ids.dart';
import '../../../../utils/api/fetch_media_file_by_ids.dart';
import '../../../../utils/api/search_collection_summary.dart';
import '../../../../utils/api/fetch_collection_group_summary_title.dart';
import '../../../../bindings/bindings.dart';
import '../../../../providers/responsive_providers.dart';
import '../../../../utils/l10n.dart';

import '../../../api/fetch_track_summary.dart';
import '../../../dialogs/mix/widgets/input_section.dart';
import '../../../dialogs/mix/widgets/editable_combo_box_section.dart';

import '../config/liked_items.dart';
import '../config/mode_select_items.dart';
import '../config/sort_order_items.dart';
import '../config/sort_select_items.dart';
import '../config/recommend_select_items.dart';

import 'number_section.dart';
import 'slider_section.dart';
import 'directory_section.dart';
import 'select_input_section.dart';
import 'mix_editor_controller.dart';
import 'select_buttons_section.dart';
import 'search_chip_input_section.dart';

class MixEditor extends StatefulWidget {
  final MixEditorController? controller;
  const MixEditor({super.key, this.controller});

  @override
  State<MixEditor> createState() => _MixEditorState();
}

class _MixEditorState extends State<MixEditor> {
  late final MixEditorController _controller;

  @override
  void initState() {
    super.initState();

    _controller = widget.controller ?? MixEditorController();
  }

  @override
  void dispose() {
    if (widget.controller == null) {
      _controller.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return SmallerOrEqualTo(
      deviceType: DeviceType.zune,
      builder: (context, isMini) {
        return ChangeNotifierProvider<MixEditorController>(
          create: (_) => _controller,
          child: ListView(
            padding: isMini ? null : const EdgeInsets.only(right: 16),
            children: [
              InputSection(
                controller: _controller.titleController,
                title: S.of(context).title,
              ),
              EditableComboBoxSection(
                controller: _controller.groupController,
                title: S.of(context).group,
                getItems: () =>
                    fetchCollectionGroupSummaryTitle(CollectionType.mix),
              ),
              SearchChipInputSection(
                controller: _controller.artistsController,
                title: S.of(context).artists,
                getInitResult: () => getInitResult(CollectionType.artist),
                searchForItems: (query) => _searchItems(
                  query,
                  'artists',
                  (x) async {
                    return (await fetchCollectionByIds(
                            CollectionType.artist, x))
                        .map((x) => (x.id, x.name))
                        .toList();
                  },
                ),
              ),
              SearchChipInputSection(
                controller: _controller.albumsController,
                title: S.of(context).albums,
                getInitResult: () => getInitResult(CollectionType.album),
                searchForItems: (query) => _searchItems(
                  query,
                  'albums',
                  (x) async {
                    return (await fetchCollectionByIds(CollectionType.album, x))
                        .map((x) => (x.id, x.name))
                        .toList();
                  },
                ),
              ),
              SearchChipInputSection(
                controller: _controller.genresController,
                title: S.of(context).genres,
                getInitResult: () => getInitResult(CollectionType.genre),
                searchForItems: (query) => _searchItems(
                  query,
                  'genres',
                  (x) async {
                    return (await fetchCollectionByIds(CollectionType.genre, x))
                        .map((x) => (x.id, x.name))
                        .toList();
                  },
                ),
              ),
              SearchChipInputSection(
                controller: _controller.playlistsController,
                title: S.of(context).playlists,
                getInitResult: () => getInitResult(CollectionType.playlist),
                searchForItems: (query) => _searchItems(
                  query,
                  'playlists',
                  (x) async {
                    return (await fetchCollectionByIds(
                      CollectionType.playlist,
                      x,
                    ))
                        .map((x) => (x.id, x.name))
                        .toList();
                  },
                ),
              ),
              SearchChipInputSection(
                controller: _controller.tracksController,
                title: S.of(context).tracks,
                getInitResult: () async {
                  return (await fetchTrackSummary())
                      .map((x) =>
                          AutoSuggestBoxItem<int>(value: x.$1, label: x.$2))
                      .toList();
                },
                searchForItems: (query) => _searchItems(
                  query,
                  'tracks',
                  (x) async {
                    return (await fetchMediaFileByIds(x, false))
                        .map((x) => (x.id, x.title))
                        .toList();
                  },
                ),
              ),
              NumberSection(
                controller: _controller.randomTrackController,
                title: S.of(context).randomTracks,
              ),
              DirectorySection(controller: _controller.directoryController),
              SliderSection(
                title: S.of(context).amount,
                controller: _controller.limitController,
              ),
              SelectInputSection(
                controller: _controller.modeController,
                title: S.of(context).mode,
                items: modeSelectItems,
                defaultValue: '99',
              ),
              SelectInputSection(
                controller: _controller.recommendationController,
                title: S.of(context).recommendation,
                items: recommendSelectItems,
                defaultValue: '',
              ),
              SelectInputSection(
                controller: _controller.sortByController,
                title: S.of(context).sortBy,
                items: sortSelectItems,
                defaultValue: 'default',
              ),
              SelectButtonsSection(
                controller: _controller.sortOrderController,
                title: S.of(context).sortOrder,
                items: sortOrderItems,
                defaultValue: 'true',
                disabled:
                    _controller.sortByController.selectedValue == 'default',
              ),
              SelectButtonsSection(
                controller: _controller.likedController,
                title: S.of(context).liked,
                items: likedItems,
                defaultValue: 'false',
              ),
            ],
          ),
        );
      },
    );
  }
}

Future<Map<String, List<int>>> searchFor(String query, String field) async {
  final searchRequest =
      SearchForRequest(queryStr: query, fields: [field], n: 30);
  searchRequest.sendSignalToRust();

  final message = (await SearchForResponse.rustSignalStream.first).message;

  final Map<String, List<int>> result = {};

  result['artists'] = message.artists;
  result['albums'] = message.albums;
  result['playlists'] = message.playlists;
  result['tracks'] = message.tracks;

  return result;
}

Future<List<AutoSuggestBoxItem<int>>> getInitResult(
  CollectionType collectionType,
) async {
  final summary = await fetchCollectionSummary(collectionType);
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
