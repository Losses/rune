import 'package:path/path.dart' as path;

import 'package:file_selector/file_selector.dart';

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
  '.wav'
];

List<String> filterAudioFiles(List<XFile> xfiles) {
  return xfiles
      .where((xfile) =>
          audioExtensions.contains(path.extension(xfile.path).toLowerCase()))
      .map((xfile) => xfile.path)
      .toList();
}
