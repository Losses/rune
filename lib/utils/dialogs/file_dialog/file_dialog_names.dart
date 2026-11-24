import '../../../bindings/bindings.dart';
import '../../../l10n/app_localizations.dart';

import 'file_dialog_icons.dart';
import 'file_dialog_model.dart';

/// How the dialog names and draws places: well-known folders in Rune's
/// language, drives by label and letter, anything else by its folder name.
class FileDialogNames {
  const FileDialogNames(this.s, this.places);

  final AppLocalizations s;
  final ListFileDialogPlacesResponse? places;

  String knownFolder(FileDialogKnownFolder folder) => switch (folder) {
    FileDialogKnownFolder.home => s.home,
    FileDialogKnownFolder.desktop => s.fileDialogDesktop,
    FileDialogKnownFolder.documents => s.fileDialogDocuments,
    FileDialogKnownFolder.downloads => s.fileDialogDownloads,
    FileDialogKnownFolder.music => s.fileDialogMusic,
    FileDialogKnownFolder.pictures => s.fileDialogPictures,
    FileDialogKnownFolder.videos => s.fileDialogVideos,
  };

  String place(FileDialogPlace place) => switch (place.name) {
    FileDialogPlaceNameKnown(:final folder) => knownFolder(folder),
    FileDialogPlaceNameCustom(:final name) => name,
    _ => nameOf(place.location),
  };

  String volume(FileDialogVolume volume) {
    final label =
        volume.label ??
        switch (volume.kind) {
          FileDialogVolumeKind.system ||
          FileDialogVolumeKind.fixed => s.fileDialogLocalDisk,
          FileDialogVolumeKind.removable => s.fileDialogRemovableDisk,
          FileDialogVolumeKind.optical => s.fileDialogOpticalDrive,
          FileDialogVolumeKind.network => s.fileDialogNetworkDrive,
        };
    // A granted Android folder is named by its label alone.
    if (volume.location.root is FileDialogRootDocumentTree) return label;
    final path = volume.location.path;
    final drive = RegExp(r'^[A-Za-z]:').stringMatch(path) ?? path;
    return s.fileDialogDriveName(label, drive);
  }

  FileDialogKnownFolder? knownFolderAt(FileDialogLocation location) {
    final places = this.places;
    if (places == null) return null;
    for (final place in [...places.folders, ...places.pinned]) {
      if (place.location == location) {
        if (place.name case FileDialogPlaceNameKnown(:final folder)) {
          return folder;
        }
      }
    }
    return null;
  }

  FileDialogVolume? volumeAt(FileDialogLocation location) => places?.volumes
      .where((volume) => volume.location == location)
      .firstOrNull;

  /// The name a location goes by in the address bar and the title of its
  /// folder.
  String location(FileDialogLocation location) {
    final volume = volumeAt(location);
    if (volume != null) return this.volume(volume);
    final known = knownFolderAt(location);
    if (known != null) return knownFolder(known);
    return nameOf(location);
  }

  /// The icon of a folder at [location].
  String folderIcon(FileDialogLocation location, double edge) {
    final volume = volumeAt(location);
    if (volume != null) return volumeIconAsset(volume, edge);
    final known = knownFolderAt(location);
    if (known != null) return knownFolderIconAsset(known, edge);
    return folderIconAsset(edge);
  }

  String entryIcon(FileDialogEntry entry, double edge) => isFolder(entry)
      ? folderIcon(entry.location, edge)
      : fileIconAsset(entry.name, edge);

  /// The type column: "File Folder", "FLAC File", or "File".
  String entryType(FileDialogEntry entry) {
    if (isFolder(entry)) return s.fileDialogFileFolder;
    final extension = extensionOf(entry.name);
    return extension.isEmpty
        ? s.fileDialogFile
        : s.fileDialogFileOfType(extension.toUpperCase());
  }
}
