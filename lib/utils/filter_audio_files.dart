import 'package:path/path.dart' as path;

import 'package:file_selector/file_selector.dart';

List<String> filterAudioFiles(List<XFile> xfiles) {
  final audioExtensions = [
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
    '.wav'
  ];

  return xfiles
      .where((xfile) =>
          audioExtensions.contains(path.extension(xfile.path).toLowerCase()))
      .map((xfile) => xfile.path)
      .toList();
}
