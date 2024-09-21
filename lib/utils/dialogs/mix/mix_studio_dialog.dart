import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/messages/mix.pbserver.dart';
import 'package:player/utils/dialogs/mix/utils.dart';
import 'package:provider/provider.dart';

import '../../../utils/query_mix_tracks.dart';
import '../../../utils/chip_input/search_task.dart';
import '../../../utils/dialogs/mix/widgets/mix_editor.dart';
import '../../../utils/dialogs/mix/utils/mix_editor_data.dart';
import '../../../utils/dialogs/mix/widgets/mix_editor_controller.dart';
import '../../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../../widgets/start_screen/providers/managed_start_screen_item.dart';
import '../../../widgets/start_screen/providers/start_screen_layout_manager.dart';
import '../../../widgets/playback_controller/constants/playback_controller_height.dart';
import '../../../screens/search/search.dart';

import '../../../messages/media_file.pb.dart';

class MixStudioDialog extends StatefulWidget {
  final int? mixId;

  const MixStudioDialog({super.key, this.mixId});

  @override
  State<MixStudioDialog> createState() => _MixStudioDialogState();
}

class _MixStudioDialogState extends State<MixStudioDialog> {
  late final _controller = MixEditorController();
  final _layoutManager = StartScreenLayoutManager();
  final _searchManager = SearchTask<MediaFile, List<(String, String)>>(
    notifyWhenStateChange: false,
    searchDelegate: queryMixTracks,
  );

  bool isLoading = false;
  String _query = '';

  @override
  void initState() {
    super.initState();
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

                      return GridView(
                        key: Key(_query),
                        gridDelegate: SliverGridDelegateWithFixedCrossAxisCount(
                          crossAxisCount: rows,
                          mainAxisSpacing: gapSize,
                          crossAxisSpacing: gapSize,
                          childAspectRatio: ratio,
                        ),
                        children: _searchManager.searchResults
                            .map((a) => TrackItem(index: 0, item: a))
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
                          _controller.modeController.selectedValue ?? '99'),
                      mixEditorDataToQuery(_controller.getData()),
                    );
                  } else {
                    response = await createMix(
                      _controller.titleController.text,
                      _controller.groupController.text,
                      false,
                      int.parse(
                          _controller.modeController.selectedValue ?? '99'),
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
