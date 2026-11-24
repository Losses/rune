import 'package:fluent_ui/fluent_ui.dart';

import '../../../utils/l10n.dart';
import '../../api/request_failure.dart';
import '../../../utils/dialogs/unavailable_dialog_on_band.dart';
import '../../../widgets/no_shortcuts.dart';
import '../../../widgets/responsive_dialog_actions.dart';
import '../../../screens/settings_library/widgets/progress_button.dart';
import '../../../bindings/bindings.dart';

/// Adds the songs of a collection to a playlist and shows how far it is. The
/// songs the playlist already has are left out. It starts when it opens and
/// closes when every song is added; closing it earlier stops the adding.
class AddToPlaylistDialog extends StatefulWidget {
  final CollectionType type;
  final int id;
  final int playlistId;

  /// The ids of the songs of a collection, in its order.
  final Future<List<int>> Function(CollectionType type, int id) loadFileIds;
  final Future<void> Function(int playlistId, int fileId) addFileToPlaylist;
  final void Function(void) $close;

  const AddToPlaylistDialog({
    super.key,
    required this.type,
    required this.id,
    required this.playlistId,
    required this.loadFileIds,
    required this.addFileToPlaylist,
    required this.$close,
  });

  @override
  AddToPlaylistDialogState createState() => AddToPlaylistDialogState();
}

class AddToPlaylistDialogState extends State<AddToPlaylistDialog> {
  bool isLoading = false;
  int added = 0;
  // How many songs are to be added, once the collection and the playlist are
  // read.
  int? total;
  RequestFailure? failure;
  bool _cancelled = false;

  /// Whether the dialog is being dismissed, by Cancel, Escape or a tap outside.
  /// It stays mounted until its exit animation ends, and sends no more adds
  /// meanwhile; an add already sent may still finish.
  bool get _dismissed => _cancelled || ModalRoute.of(context)?.isActive == false;

  @override
  void initState() {
    super.initState();
    start();
  }

  @override
  Widget build(BuildContext context) {
    final total = this.total;
    return UnavailableDialogOnBand(
      $close: widget.$close,
      child: NoShortcuts(
        ContentDialog(
          title: Text(S.of(context).addingToPlaylist),
          content: failure == null
              ? null
              : Text(requestFailureMessage(S.of(context), failure!)),
          actions: [
            ResponsiveDialogActions(
              isLoading
                  ? ProgressButton(
                      onPressed: null,
                      title: total == null
                          ? S.of(context).addingToPlaylistPreparing
                          : S
                                .of(context)
                                .addingToPlaylistProgress(added, total),
                      progress: total == null
                          ? null
                          : total == 0
                          ? 1
                          : added / total,
                      filled: true,
                    )
                  : FilledButton(
                      onPressed: start,
                      child: Text(S.of(context).continueTask),
                    ),
              Button(
                onPressed: () {
                  _cancelled = true;
                  widget.$close(null);
                },
                child: Text(S.of(context).cancel),
              ),
            ),
          ],
        ),
      ),
    );
  }

  /// Adds the songs the playlist does not have yet, one by one. It does
  /// nothing while a run is under way.
  Future<void> start() async {
    if (isLoading) return;
    setState(() {
      isLoading = true;
      failure = null;
      added = 0;
      total = null;
    });

    try {
      final songs = await widget.loadFileIds(widget.type, widget.id);
      final present = (await widget.loadFileIds(
        CollectionType.playlist,
        widget.playlistId,
      )).toSet();
      // A song the collection lists twice is added once, at its first place.
      final seen = {...present};
      final missing = songs.where(seen.add).toList();
      if (!mounted) return;
      setState(() => total = missing.length);

      for (final fileId in missing) {
        // A dialog that is going away sends no more.
        if (_dismissed) return;
        await widget.addFileToPlaylist(widget.playlistId, fileId);
        // A closed dialog adds no more.
        if (!mounted) return;
        setState(() => added++);
      }

      widget.$close(null);
    } on RequestFailure catch (error) {
      if (mounted) {
        setState(() {
          failure = error;
          isLoading = false;
        });
      }
    } catch (error) {
      // A request that got no answer in time throws something else, and the
      // dialog must not stay on its progress.
      if (mounted) {
        setState(() {
          failure = RequestFailure(
            'AddItemToPlaylistRequest',
            RequestErrorKind.internal,
            '$error',
          );
          isLoading = false;
        });
      }
    }
  }
}
