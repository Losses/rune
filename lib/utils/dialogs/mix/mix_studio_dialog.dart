import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../screens/search/search.dart';

import '../../../messages/mix.pbserver.dart';
import '../../../messages/media_file.pb.dart';

import '../../query_list.dart';
import '../../chip_input/search_task.dart';
import '../../api/create_mix.dart';
import '../../api/update_mix.dart';
import '../../api/get_mix_by_id.dart';
import '../../api/query_mix_tracks.dart';
import '../../api/fetch_mix_queries_by_mix_id.dart';
import '../../dialogs/mix/widgets/mix_editor.dart';
import '../../dialogs/mix/utils/mix_editor_data.dart';
import '../../dialogs/mix/widgets/mix_editor_controller.dart';

class MixStudioDialog extends StatefulWidget {
  final int? mixId;

  const MixStudioDialog({super.key, required this.mixId});

  @override
  State<MixStudioDialog> createState() => _MixStudioDialogState();
}

class _MixStudioDialogState extends State<MixStudioDialog> {
  late final _controller = MixEditorController();
  final _layoutManager = StartScreenLayoutManager();
  final _searchManager = SearchTask<MediaFile, List<(String, String)>>(
    notifyWhenStateChange: false,
    searchDelegate: (x) => queryMixTracks(QueryList(x)),
  );

  bool isLoading = false;
  String _query = '';

  @override
  void initState() {
    super.initState();

    if (widget.mixId != null) {
      loadMix(widget.mixId!);
    }

    _controller.addListener(() {
      _layoutManager.resetAnimations();
      _searchManager.search(mixEditorDataToQuery(_controller.getData()));
    });
    _searchManager.addListener(() {
      setState(() {
        final query = mixEditorDataToQuery(_controller.getData());

        _query = query.map((x) => '$x').join(';');
      });
      _layoutManager.playAnimations();
    });
  }

  Future<void> loadMix(int mixId) async {
    final mix = await getMixById(mixId);
    final queries = await fetchMixQueriesByMixId(mixId);

    final queryData = await queryToMixEditorData(mix.name, mix.group, queries);

    _controller.setData(queryData);
    setState(() {});
  }

  @override
  void dispose() {
    _controller.dispose();
    _layoutManager.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final height = MediaQuery.of(context).size.height;
    const reduce = navigationBarHeight + playbackControllerHeight + 48;

    return ContentDialog(
      constraints: const BoxConstraints(maxWidth: 1000),
      title: Column(
        children: [
          const SizedBox(height: 8),
          Text(widget.mixId != null ? 'Edit Mix' : 'Create Mix'),
        ],
      ),
      content: Container(
        constraints: BoxConstraints(
          maxHeight: height < reduce ? reduce : height - reduce,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: 380,
              child: SizedBox(
                height: height - reduce,
                child: MixEditor(controller: _controller),
              ),
            ),
            const SizedBox(width: 6),
            Expanded(
              child: SizedBox(
                height: height - reduce,
                child: ChangeNotifierProvider<StartScreenLayoutManager>.value(
                  value: _layoutManager,
                  child: LayoutBuilder(
                    builder: (context, constraints) {
                      const double gapSize = 8;
                      const double cellSize = 200;

                      const ratio = 4 / 1;

                      final int rows =
                          (constraints.maxWidth / (cellSize + gapSize)).floor();

                      final trackIds =
                          _searchManager.searchResults.map((x) => x.id).toList();

                      return GridView(
                        key: Key(_query),
                        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                          crossAxisCount: rows,
                          mainAxisSpacing: gapSize,
                          crossAxisSpacing: gapSize,
                          childAspectRatio: ratio,
                        ),
                        children: _searchManager.searchResults
                            .map((a) => TrackSearchItem(
                                  index: 0,
                                  item: a,
                                  fallbackFileIds: trackIds,
                                ))
                            .toList()
                            .asMap()
                            .entries
                            .map((x) {
                          final index = x.key;
                          final int row = index % rows;
                          final int column = index ~/ rows;

                          return ManagedStartScreenItem(
                            key: Key('$row:$column'),
                            prefix: _query,
                            groupId: 0,
                            row: row,
                            column: column,
                            width: cellSize / ratio,
                            height: cellSize,
                            child: x.value,
                          );
                        }).toList(),
                      );
                    },
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
      actions: [
        FilledButton(
          onPressed: isLoading
              ? null
              : () async {
                  setState(() {
                    isLoading = true;
                  });

                  MixWithoutCoverIds? response;
                  if (widget.mixId != null) {
                    response = await updateMix(
                      widget.mixId!,
                      _controller.titleController.text,
                      _controller.groupController.text,
                      false,
                      int.parse(
                        _controller.modeController.selectedValue ?? '99',
                      ),
                      mixEditorDataToQuery(_controller.getData()),
                    );
                  } else {
                    response = await createMix(
                      _controller.titleController.text,
                      _controller.groupController.text,
                      false,
                      int.parse(
                        _controller.modeController.selectedValue ?? '99',
                      ),
                      mixEditorDataToQuery(_controller.getData()),
                    );
                  }

                  setState(() {
                    isLoading = false;
                  });

                  if (!context.mounted) return;
                  Navigator.pop(context, response);
                },
          child: Text(widget.mixId != null ? 'Save' : 'Create'),
        ),
        Button(
          onPressed: isLoading ? null : () => Navigator.pop(context, null),
          child: const Text('Cancel'),
        ),
      ],
    );
  }
}
