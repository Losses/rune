import 'package:fluent_ui/fluent_ui.dart';
import 'package:fast_file_picker/fast_file_picker.dart';
import 'package:file_selector/file_selector.dart' show XTypeGroup;

import '../../bindings/bindings.dart';

import '../query_list.dart';
import '../playing_item.dart';
import '../filter_audio_files.dart';
import '../api/operate_playback_with_mix_query.dart';

import 'open_intent.dart';

class OpenAction extends Action<OpenIntent> {
  OpenAction();

  @override
  void invoke(covariant OpenIntent intent) async {
    const XTypeGroup typeGroup = XTypeGroup(
      label: 'audio files',
      extensions: audioExtensions,
    );

    final List<FastFilePickerPath>? files =
        await FastFilePicker.pickMultipleFiles(
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

    operatePlaybackWithMixQuery(
      queries: QueryList(),
      playbackMode: 99,
      hintPosition: -1,
      initialPlaybackId: -1,
      instantlyPlay: true,
      operateMode: PlaylistOperateMode.replace,
      fallbackPlayingItems: items,
    );
  }
}
