import 'package:fluent_ui/fluent_ui.dart';
import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../bindings/bindings.dart';

import '../query_list.dart';
import '../playing_item.dart';
import '../filter_audio_files.dart';
import '../api/operate_playback_with_mix_query.dart';

import 'open_intent.dart';
import '../dialogs/file_dialog/pick_files.dart';

class OpenAction extends ContextAction<OpenIntent> {
  OpenAction();

  @override
  void invoke(covariant OpenIntent intent, [BuildContext? context]) async {
    const XTypeGroup typeGroup = XTypeGroup(
      label: 'audio files',
      extensions: audioExtensions,
    );

    if (context == null || !context.mounted) return;
    final List<FastFilePickerPath>? files = await pickMultipleFiles(
      context,
      acceptedTypeGroups: <XTypeGroup>[typeGroup],
    );

    if (files == null) {
      return;
    }

    final items = filterAudioFiles(
      files,
    ).map(PlayingItem.independentFile).toList();

    if (items.isEmpty) {
      return;
    }

    if (!context.mounted) return;
    await safeOperatePlaybackWithMixQuery(
      context: context,
      queries: QueryList(),
      playbackMode: null,
      hintPosition: -1,
      initialPlaybackId: null,
      instantlyPlay: true,
      operateMode: PlaylistOperateMode.replace,
      fallbackPlayingItems: items,
    );
  }
}
