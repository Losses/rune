import '../../api/request_failure.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:provider/provider.dart';

import '../../../providers/library_registry.dart';
import '../../../utils/l10n.dart';
import '../../../utils/api/update_playlist.dart';
import '../../../utils/api/create_playlist.dart';
import '../../../utils/api/get_playlist_by_id.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../screens/collection/utils/collection_data_provider.dart';
import '../../../bindings/bindings.dart';

import '../../api/create_m3u8_playlist.dart';
import '../../api/fetch_collection_group_summary_title.dart';

import '../clean_group_titles.dart';
import 'show_import_m3u8_success_dialog.dart';

class CreateEditPlaylistDialog extends StatefulWidget {
  final int? playlistId;
  final String? defaultTitle;
  final String? m3u8Path;
  final void Function(Playlist?) $close;

  const CreateEditPlaylistDialog({
    super.key,
    this.playlistId,
    required this.defaultTitle,
    this.m3u8Path,
    required this.$close,
  });

  @override
  CreateEditPlaylistDialogState createState() =>
      CreateEditPlaylistDialogState();
}

class CreateEditPlaylistDialogState extends State<CreateEditPlaylistDialog> {
  final titleController = TextEditingController();
  final groupController = TextEditingController();
  bool isLoading = false;
  RequestFailure? failure;
  List<String> groupList = ['Favorite'];

  Playlist? playlist;
  String? _playlistRecordId;
  bool _needsPlaylistRead = true;

  @override
  void initState() {
    super.initState();

    final defaultTitle = widget.defaultTitle;

    if (defaultTitle != null) {
      titleController.text = defaultTitle;
    }

    fetchGroupList();
    if (widget.playlistId != null) {
      _playlistRecordId = context
          .read<LibraryRegistryProvider?>()
          ?.openRecordId;
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _loadPlaylistIfReady();
  }

  @override
  dispose() {
    super.dispose();
    titleController.dispose();
    groupController.dispose();
  }

  Future<void> fetchGroupList() async {
    try {
      final groups = await fetchCollectionGroupSummaryTitle(
        CollectionType.playlist,
      );
      if (!mounted) return;
      setState(() {
        groupList = cleanGroupTitles(['Favorite', ...groups]);
      });
    } on RequestFailure catch (error) {
      if (mounted) setState(() => failure ??= error);
    }
  }

  void _loadPlaylistIfReady() {
    final id = widget.playlistId;
    if (!_needsPlaylistRead || id == null) return;
    final registry = context.read<LibraryRegistryProvider?>();
    if (registry?.isOpening == true ||
        registry?.openRecordId != _playlistRecordId) {
      return;
    }
    _needsPlaylistRead = false;
    _loadPlaylist(id, registry);
  }

  Future<void> _loadPlaylist(
    int playlistId,
    LibraryRegistryProvider? registry,
  ) async {
    final generation = registry?.openGeneration;
    setState(() => isLoading = true);
    Playlist? result;
    RequestFailure? readFailure;
    try {
      result = await getPlaylistById(playlistId);
    } on RequestFailure catch (error) {
      readFailure = error;
    }
    if (!mounted) return;
    if (registry?.isOpening == true ||
        registry?.openRecordId != _playlistRecordId ||
        registry?.openGeneration != generation) {
      setState(() {
        isLoading = false;
        _needsPlaylistRead = true;
      });
      _loadPlaylistIfReady();
      return;
    }
    setState(() {
      isLoading = false;
      if (readFailure != null) {
        failure = readFailure;
      } else if (result != null) {
        playlist = result;
        if (titleController.text == (widget.defaultTitle ?? '')) {
          titleController.text = result.name;
        }
        if (groupController.text.isEmpty) groupController.text = result.group;
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final registry = widget.playlistId == null
        ? null
        : context.watch<LibraryRegistryProvider?>();
    // Playlist IDs are local to the library that served the read.
    bool canSave() =>
        !isLoading &&
        (widget.playlistId == null ||
            (playlist != null &&
                registry?.isOpening != true &&
                registry?.openRecordId == _playlistRecordId));
    return UnavailableDialogOnBand(
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
          title: Column(
            children: [
              const SizedBox(height: 8),
              Text(
                widget.playlistId != null
                    ? S.of(context).editPlaylist
                    : S.of(context).createPlaylist,
              ),
            ],
          ),
          content: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (failure?.request == 'CreateM3u8PlaylistRequest')
                Text(S.of(context).importM3u8Failed),
              if (failure != null)
                Text(requestFailureMessage(S.of(context), failure!))
              else if (!isLoading && !canSave())
                Text(S.of(context).requestUnavailable),
              const SizedBox(height: 16),
              InfoLabel(
                label: S.of(context).title,
                child: TextBox(
                  controller: titleController,
                  enabled: !isLoading,
                ),
              ),
              const SizedBox(height: 16),
              InfoLabel(
                label: S.of(context).group,
                child: AutoSuggestBox<String>(
                  controller: groupController,
                  items: groupList.map<AutoSuggestBoxItem<String>>((e) {
                    return AutoSuggestBoxItem<String>(value: e, label: e);
                  }).toList(),
                  placeholder: S.of(context).selectAGroup,
                ),
              ),
              const SizedBox(height: 8),
            ],
          ),
          actions: [
            ResponsiveDialogActions(
              FilledButton(
                onPressed: !canSave()
                    ? null
                    : () async {
                        if (!canSave()) return;
                        setState(() {
                          isLoading = true;
                        });

                        try {
                          Playlist? response;
                          if (widget.m3u8Path != null) {
                            final result = await createM3u8Playlist(
                              titleController.text,
                              groupController.text,
                              widget.m3u8Path!,
                            );
                            if (!context.mounted) return;
                            widget.$close(result.playlist);
                            CollectionCache().clearType(
                              CollectionType.playlist,
                            );
                            showM3u8ImportResultDialog(context, result);
                            return;
                          } else if (widget.playlistId != null) {
                            response = await updatePlaylist(
                              widget.playlistId!,
                              titleController.text,
                              groupController.text,
                            );
                          } else {
                            response = await createPlaylist(
                              titleController.text,
                              groupController.text,
                            );
                          }
                          CollectionCache().clearType(CollectionType.playlist);
                          if (!mounted) return;
                          setState(() => isLoading = false);
                          widget.$close(response);
                        } on RequestFailure catch (error) {
                          if (mounted) {
                            setState(() {
                              failure = error;
                              isLoading = false;
                            });
                          }
                        }
                      },
                child: Text(
                  widget.playlistId != null
                      ? S.of(context).save
                      : S.of(context).create,
                ),
              ),
              Button(
                onPressed: isLoading ? null : () => widget.$close(null),
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
