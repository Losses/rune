import 'package:player/messages/mix.pbserver.dart';
import 'package:player/messages/playback.pb.dart';
import 'package:player/utils/api/add_item_to_mix.dart';
import 'package:player/utils/api/add_item_to_playlist.dart';
import 'package:player/utils/api/get_all_mixes.dart';
import 'package:player/utils/dialogs/mix/create_edit_mix.dart';
import 'package:provider/provider.dart';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../utils/router_extra.dart';
import '../../utils/api/get_all_playlists.dart';
import '../../utils/api/if_analysis_exists.dart';
import '../../utils/api/get_parsed_media_file.dart';
import '../../screens/settings_library/widgets/progress_button.dart';
import '../../messages/media_file.pb.dart';
import '../../messages/playlist.pbserver.dart';
import '../../providers/library_path.dart';
import '../../providers/library_manager.dart';

import '../dialogs/playlist/create_edit_playlist.dart';

void openTrackItemContextMenu(
    Offset localPosition,
    BuildContext context,
    GlobalKey contextAttachKey,
    FlyoutController contextController,
    int fileId) async {
  final targetContext = contextAttachKey.currentContext;

  if (targetContext == null) return;
  final box = targetContext.findRenderObject() as RenderBox;
  final position = box.localToGlobal(
    localPosition,
    ancestor: Navigator.of(context).context.findRenderObject(),
  );
  final analysed = await ifAnalysisExists(fileId);

  final playlists = await getAllPlaylists();
  final mixes = await getAllMixes();
  final parsedMediaFile = await getParsedMediaFile(fileId);

  contextController.showFlyout(
    position: position,
    builder: (context) => buildTrackItemContextMenu(
      context,
      parsedMediaFile,
      playlists,
      mixes,
      analysed,
    ),
  );
}

Future<String?> showNoAnalysisDialog(BuildContext context) async {
  return showDialog<String>(
    context: context,
    builder: (context) => ContentDialog(
      title: const Column(
        children: [
          SizedBox(height: 8),
          Text("Not Ready"),
        ],
      ),
      content: const Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          NotAnalysedText(),
          SizedBox(height: 4),
        ],
      ),
      actions: [
        const AnalysisActionButton(),
        Button(
          child: const Text('Cancel'),
          onPressed: () => Navigator.pop(context, 'Cancel'),
        ),
      ],
    ),
  );
}

class NotAnalysedText extends StatelessWidget {
  const NotAnalysedText({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final itemPath = libraryPath.currentPath ?? '';

    final scanProgress = libraryManager.getScanTaskProgress(itemPath);
    final analyseProgress = libraryManager.getAnalyseTaskProgress(itemPath);

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyseWorking = analyseProgress?.status == TaskStatus.working;

    if (scanWorking) {
      return const Text(
        "Unable to start roaming. This track hasn't been analyzed yet. The library is being scanned, so analysis cannot be performed.",
      );
    }

    if (analyseWorking) {
      return const Text(
        "Unable to start roaming. This track hasn't been analyzed yet. The library is being analyzed; please wait until the process finished.",
      );
    }

    return const Text(
      "Unable to start roaming. This track hasn't been analyzed yet. Please analyze your library for the best experience.",
    );
  }
}

class AnalysisActionButton extends StatelessWidget {
  const AnalysisActionButton({
    super.key,
  });

  @override
  Widget build(BuildContext context) {
    final libraryManager =
        Provider.of<LibraryManagerProvider>(context, listen: true);
    final libraryPath = Provider.of<LibraryPathProvider>(context, listen: true);
    final itemPath = libraryPath.currentPath ?? '';

    final scanProgress = libraryManager.getScanTaskProgress(itemPath);
    final analyseProgress = libraryManager.getAnalyseTaskProgress(itemPath);

    final scanWorking = scanProgress?.status == TaskStatus.working;
    final analyseWorking = analyseProgress?.status == TaskStatus.working;

    if (scanWorking) {
      return const FilledButton(
        onPressed: null,
        child: Text('Analysis'),
      );
    }

    if (analyseWorking) {
      return const ProgressButton(
        title: "Analysing",
        onPressed: null,
      );
    }

    return FilledButton(
      onPressed: () {
        libraryManager.analyseLibrary(itemPath, false);
        Navigator.pop(context, 'Analysis');
      },
      child: const Text("Analyse"),
    );
  }
}

Widget buildTrackItemContextMenu(
  BuildContext context,
  FetchParsedMediaFileResponse item,
  List<PlaylistWithoutCoverIds> playlists,
  List<MixWithoutCoverIds> mixes,
  bool analysed,
) {
  final List<MenuFlyoutItem> playlistItems = playlists.map((playlist) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.list_alt),
      text: Text(playlist.name),
      onPressed: () {
        addItemToPlaylist(playlist.id, item.file.id);

        Flyout.of(context).close();
      },
    );
  }).toList();

  final List<MenuFlyoutItem> mixItems =
      mixes.where((x) => !x.locked).map((mixes) {
    return MenuFlyoutItem(
      leading: const Icon(Symbols.magic_button),
      text: Text(mixes.name),
      onPressed: () {
        addItemToMix(
          mixes.id,
          "lib::track",
          item.file.id.toString(),
        );

        Flyout.of(context).close();
      },
    );
  }).toList();

  return MenuFlyout(
    items: [
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () {
          OperatePlaybackWithMixQueryRequest(
            queries: [
              MixQuery(
                  operator: "lib::track", parameter: item.file.id.toString()),
              MixQuery(operator: "pipe::limit", parameter: "50"),
              MixQuery(operator: "pipe::recommend", parameter: "-1")
            ],
            playbackMode: 99,
            hintPosition: -1,
            initialPlaybackId: 0,
            instantlyPlay: true,
            replacePlaylist: true,
          ).sendSignalToRust();
        },
      ),
      MenuFlyoutItem(
        leading: const Icon(Symbols.rocket),
        text: const Text('Start Roaming'),
        onPressed: () async {
          await showNoAnalysisDialog(context);
        },
      ),
      if (item.artists.length == 1)
        MenuFlyoutItem(
          leading: const Icon(Symbols.face),
          text: const Text('Go to Artist'),
          onPressed: () => {
            GoRouter.of(context).replace('/artists/${item.artists[0].id}',
                extra: QueryTracksExtra(item.artists[0].name))
          },
        ),
      if (item.artists.length > 1)
        MenuFlyoutSubItem(
            leading: const Icon(Symbols.face),
            text: const Text('Go to Artist'),
            items: (context) => item.artists
                .map((x) => MenuFlyoutItem(
                      leading: const Icon(Symbols.face),
                      text: Text(x.name),
                      onPressed: () => {
                        GoRouter.of(context).replace('/artists/${x.id}',
                            extra: QueryTracksExtra(x.name))
                      },
                    ))
                .toList()),
      MenuFlyoutItem(
        leading: const Icon(Symbols.album),
        text: const Text('Go to Album'),
        onPressed: () => {
          GoRouter.of(context).replace(
            '/albums/${item.album.id}',
            extra: QueryTracksExtra(item.album.name),
          )
        },
      ),
      const MenuFlyoutSeparator(),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.list_alt),
        text: const Text('Add to Playlist'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Playlist'),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist =
                  await showCreateEditPlaylistDialog(context, playlistId: null);

              if (playlist == null) return;

              await addItemToPlaylist(playlist.id, item.file.id);
            },
          ),
          if (playlistItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...playlistItems
        ],
      ),
      MenuFlyoutSubItem(
        leading: const Icon(Symbols.magic_button),
        text: const Text('Add to Mix'),
        items: (context) => [
          MenuFlyoutItem(
            leading: const Icon(Symbols.add),
            text: const Text('New Mix'),
            onPressed: () async {
              Flyout.of(context).close();

              final playlist =
                  await showCreateEditMixDialog(context, mixId: null);

              if (playlist == null) return;

              await addItemToMix(
                playlist.id,
                "lib::track",
                item.file.id.toString(),
              );
            },
          ),
          if (mixItems.isNotEmpty) const MenuFlyoutSeparator(),
          ...mixItems
        ],
      ),
    ],
  );
}
