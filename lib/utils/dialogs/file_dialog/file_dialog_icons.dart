import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_svg/flutter_svg.dart';

import '../../../bindings/bindings.dart';

import 'file_dialog_model.dart';

/// The folder of `assets/mime` each icon lives in, by its name.
const Map<String, String> _iconFolders = {
  // archive
  '7z': 'archive', 'arc': 'archive', 'bz2': 'archive', 'bz': 'archive',
  'gz': 'archive', 'jar': 'archive', 'mpkg': 'archive', 'rar': 'archive',
  'tar': 'archive', 'zip': 'archive',
  // archive2
  'ace': 'archive2', 'lha': 'archive2', 'mscompress': 'archive2',
  'squashfs': 'archive2', 'xz': 'archive2', 'zlibstream': 'archive2',
  // audio
  'aac': 'audio', 'aiff': 'audio', 'alac': 'audio', 'ape': 'audio',
  'dsd': 'audio', 'flac': 'audio', 'midi': 'audio', 'mp3': 'audio',
  'opus': 'audio', 'vorbis': 'audio', 'wavpack': 'audio', 'wav': 'audio',
  'wma': 'audio',
  // binary
  'cab': 'binary', 'class': 'binary', 'coff': 'binary', 'crx': 'binary',
  'dex': 'binary', 'elf': 'binary', 'macho': 'binary', 'nupkg': 'binary',
  'pdb': 'binary', 'pyc': 'binary', 'xar': 'binary', 'xpi': 'binary',
  // code
  'csh': 'code', 'css': 'code', 'html': 'code', 'jsonld': 'code',
  'json': 'code', 'js': 'code', 'mjs': 'code', 'php': 'code', 'sh': 'code',
  'xhtml': 'code', 'xml': 'code', 'xul': 'code',
  // docmisc
  'bib': 'docmisc', 'chm': 'docmisc', 'eml': 'docmisc', 'hlp': 'docmisc',
  'mht': 'docmisc', 'msg': 'docmisc', 'one': 'docmisc', 'po': 'docmisc',
  'ps': 'docmisc', 'rst': 'docmisc', 'tex': 'docmisc', 'torrent': 'docmisc',
  'tsv': 'docmisc', 'xlsb': 'docmisc',
  // document
  'abw': 'document', 'azw': 'document', 'doc': 'document',
  'docx': 'document', 'epub': 'document', 'odt': 'document',
  'pdf': 'document', 'rtf': 'document',
  // executable
  'apk': 'executable', 'appimage': 'executable', 'deb': 'executable',
  'desktop': 'executable', 'dll': 'executable', 'dmg': 'executable',
  'exe': 'executable', 'flatpak': 'executable', 'ipa': 'executable',
  'iso': 'executable', 'msi': 'executable', 'rpm': 'executable',
  'snap': 'executable', 'so': 'executable',
  // font
  'eot': 'font', 'otf': 'font', 'ttf': 'font', 'woff2': 'font',
  'woff': 'font',
  // image
  'apng': 'image', 'avif': 'image', 'bmp': 'image', 'gif': 'image',
  'ico': 'image', 'jpg': 'image', 'png': 'image', 'svg': 'image',
  'tiff': 'image', 'webp': 'image',
  // media2
  'flv': 'media2', 'm3u': 'media2', 'mkv': 'media2', 'mov': 'media2',
  'ogg': 'media2', 'srt': 'media2', 'swf': 'media2', 'vtt': 'media2',
  // misc
  'bin': 'misc', 'cda': 'misc', 'ics': 'misc', 'md': 'misc', 'oga': 'misc',
  'ogx': 'misc', 'txt': 'misc', 'wasm': 'misc', 'weba': 'misc',
  'webmanifest': 'misc',
  // office
  'csv': 'office', 'odp': 'office', 'ods': 'office', 'ppt': 'office',
  'pptx': 'office', 'vsd': 'office', 'xls': 'office', 'xlsx': 'office',
  // video
  '3g2': 'video', '3gp': 'video', 'avi': 'video', 'mp4': 'video',
  'mpeg': 'video', 'ogv': 'video', 'ts': 'video', 'webm': 'video',
};

/// Extensions whose icon carries another name.
const Map<String, String> _iconAliases = {
  'aif': 'aiff',
  'aifc': 'aiff',
  'dsf': 'dsd',
  'dff': 'dsd',
  'mid': 'midi',
  'wv': 'wavpack',
  'm4a': 'aac',
  'm4b': 'aac',
  'lzh': 'lha',
  'tgz': 'gz',
  'htm': 'html',
  'mhtml': 'mht',
  'bash': 'sh',
  'zsh': 'sh',
  'jpeg': 'jpg',
  'jfif': 'jpg',
  'tif': 'tiff',
  'm3u8': 'm3u',
  'pls': 'm3u',
  'm4v': 'mp4',
  'mpg': 'mpeg',
  'markdown': 'md',
  'dylib': 'macho',
  'obj': 'coff',
  // Text a music library keeps beside its tracks.
  'cue': 'txt',
  'lrc': 'txt',
  'log': 'txt',
  'nfo': 'txt',
  'ini': 'txt',
  'cfg': 'txt',
  'conf': 'txt',
};

/// Asset size folders, smallest first; each draws its own level of detail.
const List<int> _iconSizes = [16, 32, 128];

/// The asset size drawn for an icon [edge] logical pixels wide: the detail
/// level made for that size.
int _assetSize(double edge) => edge <= 24
    ? _iconSizes[0]
    : edge <= 64
    ? _iconSizes[1]
    : _iconSizes[2];

String _asset(String folder, String name, double edge) =>
    'assets/mime/$folder/${_assetSize(edge)}/$name.svg';

/// The icon of a file called [name]; a plain document for unknown types.
String fileIconAsset(String name, double edge) {
  final extension = extensionOf(name);
  final icon = _iconAliases[extension] ?? extension;
  final folder = _iconFolders[icon];
  return folder == null
      ? _asset('misc', 'txt', edge)
      : _asset(folder, icon, edge);
}

String folderIconAsset(double edge) => _asset('places', 'folder', edge);

String knownFolderIconAsset(FileDialogKnownFolder folder, double edge) =>
    _asset('places', switch (folder) {
      FileDialogKnownFolder.home => 'user-home',
      FileDialogKnownFolder.desktop => 'folder-desktop',
      FileDialogKnownFolder.documents => 'folder-documents',
      FileDialogKnownFolder.downloads => 'folder-download',
      FileDialogKnownFolder.music => 'folder-music',
      FileDialogKnownFolder.pictures => 'folder-pictures',
      FileDialogKnownFolder.videos => 'folder-videos',
    }, edge);

String volumeIconAsset(FileDialogVolume volume, double edge) =>
    _asset('places', switch (volume.kind) {
      FileDialogVolumeKind.system ||
      FileDialogVolumeKind.fixed => 'drive-harddisk',
      FileDialogVolumeKind.removable =>
        volume.location.root is FileDialogRootDocumentTree
            ? 'media-memory-sd'
            : 'drive-removable-media-usb',
      FileDialogVolumeKind.optical => 'drive-optical',
      FileDialogVolumeKind.network => 'network-drive',
    }, edge);

String thisDeviceIconAsset(double edge) => _asset('places', 'computer', edge);

/// An icon asset drawn at [size] logical pixels.
class FileDialogAssetIcon extends StatelessWidget {
  const FileDialogAssetIcon(this.asset, {super.key, required this.size});

  final String asset;
  final double size;

  @override
  Widget build(BuildContext context) => SvgPicture.asset(
    asset,
    width: size,
    height: size,
    excludeFromSemantics: true,
  );
}
