import 'package:path/path.dart' as path;

import 'package:fast_file_picker/fast_file_picker.dart';

const audioExtensions = [
  '.aac',
  '.aiff',
  '.alac',
  '.adpcm',
  '.wav',
  '.flac',
  '.m4a',
  '.mp3',
  '.pcm',
  '.ogg',
  '.vorbis',
  '.opus',
  '.wav',
];

List<String> filterAudioFiles(List<FastFilePickerPath> files) {
  return files
      .where(
        (xfile) =>
            audioExtensions.contains(path.extension(xfile.name).toLowerCase()),
      )
      .map((file) => file.path ?? file.uri)
      .whereType<String>()
      .toList();
}
