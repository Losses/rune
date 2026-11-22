import '../../../bindings/bindings.dart';

/// The layouts of the item area, in the order File Explorer lists them, each
/// with the shortcut that picks it (Ctrl+Shift+1 to 8).
enum FileDialogViewMode {
  extraLargeIcons(256),
  largeIcons(96),
  mediumIcons(48),
  smallIcons(16),
  list(16),
  details(16),
  tiles(48),
  content(32);

  const FileDialogViewMode(this.iconSize);

  /// The edge of an item's icon in logical pixels.
  final double iconSize;

  /// Whether items are laid out in a grid of cells rather than rows.
  bool get isGrid =>
      this == extraLargeIcons || this == largeIcons || this == mediumIcons;

  /// Whether the view is large enough to show thumbnails instead of icons.
  bool get showsThumbnails => iconSize >= 32;
}

enum FileDialogSortColumn { name, dateModified, type, size }

class FileDialogSort {
  const FileDialogSort(this.column, this.ascending);

  final FileDialogSortColumn column;
  final bool ascending;

  /// Sorting by the same column again flips the order, as column headers do.
  FileDialogSort toggled(FileDialogSortColumn next) => next == column
      ? FileDialogSort(column, !ascending)
      : FileDialogSort(next, true);
}

/// The extension of [name] without the dot, lowercased, or empty when it has
/// none. A leading dot alone (".profile") is a hidden name, not an extension.
String extensionOf(String name) {
  final dot = name.lastIndexOf('.');
  if (dot <= 0 || dot == name.length - 1) return '';
  return name.substring(dot + 1).toLowerCase();
}

/// The name without its extension, which inline renaming selects.
String stemOf(String name) {
  final extension = extensionOf(name);
  return extension.isEmpty
      ? name
      : name.substring(0, name.length - extension.length - 1);
}

bool isFolder(FileDialogEntry entry) =>
    entry.kind == FileDialogEntryKind.folder;

/// Items in display order: folders first, then by [sort], names compared
/// as people read them ("Track 2" before "Track 10").
List<FileDialogEntry> sortEntries(
  Iterable<FileDialogEntry> entries,
  FileDialogSort sort,
) {
  int byName(FileDialogEntry a, FileDialogEntry b) =>
      compareNatural(a.name, b.name);
  int byColumn(FileDialogEntry a, FileDialogEntry b) {
    final order = switch (sort.column) {
      FileDialogSortColumn.name => byName(a, b),
      FileDialogSortColumn.dateModified => (a.modified?.toInt() ?? 0).compareTo(
        b.modified?.toInt() ?? 0,
      ),
      FileDialogSortColumn.type => extensionOf(
        a.name,
      ).compareTo(extensionOf(b.name)),
      FileDialogSortColumn.size => a.size.toInt().compareTo(b.size.toInt()),
    };
    return order != 0 ? order : byName(a, b);
  }

  final sorted = entries.toList()
    ..sort((a, b) {
      final folders = (isFolder(a) ? 0 : 1).compareTo(isFolder(b) ? 0 : 1);
      if (folders != 0) return folders;
      final order = byColumn(a, b);
      return sort.ascending ? order : -order;
    });
  return sorted;
}

/// [a] and [b] compared case-insensitively, with runs of digits compared by
/// their value.
int compareNatural(String a, String b) {
  final left = a.toLowerCase();
  final right = b.toLowerCase();
  var i = 0;
  var j = 0;
  bool digit(int unit) => unit >= 0x30 && unit <= 0x39;
  while (i < left.length && j < right.length) {
    final x = left.codeUnitAt(i);
    final y = right.codeUnitAt(j);
    if (digit(x) && digit(y)) {
      final startI = i;
      final startJ = j;
      while (i < left.length && digit(left.codeUnitAt(i))) {
        i++;
      }
      while (j < right.length && digit(right.codeUnitAt(j))) {
        j++;
      }
      final numberA = BigInt.parse(left.substring(startI, i));
      final numberB = BigInt.parse(right.substring(startJ, j));
      final order = numberA.compareTo(numberB);
      if (order != 0) return order;
      continue;
    }
    if (x != y) return x.compareTo(y);
    i++;
    j++;
  }
  final remaining = (left.length - i).compareTo(right.length - j);
  return remaining != 0 ? remaining : a.compareTo(b);
}

/// A file size the way File Explorer's details show it: whole kilobytes,
/// rounded up, so a one-byte file reads 1 KB.
String detailsSize(int bytes) {
  final kilobytes = (bytes + 1023) ~/ 1024;
  return '${_groupDigits(kilobytes)} KB';
}

/// A file size in the largest unit that keeps it above one, as tiles and
/// drive bars show it.
String shortSize(int bytes) {
  const units = ['bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];
  if (bytes < 1024) return '$bytes ${units.first}';
  var value = bytes.toDouble();
  var unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  final digits = value >= 100 ? 0 : (value >= 10 ? 1 : 2);
  return '${value.toStringAsFixed(digits)} ${units[unit]}';
}

String _groupDigits(int value) {
  final text = value.toString();
  final buffer = StringBuffer();
  for (var index = 0; index < text.length; index++) {
    if (index > 0 && (text.length - index) % 3 == 0) buffer.write(',');
    buffer.write(text[index]);
  }
  return buffer.toString();
}

/// The parent folder of [location], or null at the top of its root.
FileDialogLocation? parentOf(FileDialogLocation location) {
  final path = location.path;
  final trimmed = _trimSeparators(path);
  final separator = trimmed.lastIndexOf(RegExp(r'[/\\]'));
  if (separator < 0) {
    // A document tree's top is the empty path.
    if (location.root is FileDialogRootDocumentTree && trimmed.isNotEmpty) {
      return FileDialogLocation(root: location.root, path: '');
    }
    return null;
  }
  var parent = trimmed.substring(0, separator);
  // "C:" alone is a drive's current folder, its root is "C:\".
  if (RegExp(r'^[A-Za-z]:$').hasMatch(parent)) parent = '$parent\\';
  if (parent.isEmpty) parent = trimmed.startsWith('/') ? '/' : '';
  if (parent == path) return null;
  return FileDialogLocation(root: location.root, path: parent);
}

String _trimSeparators(String path) {
  var end = path.length;
  while (end > 1 && (path[end - 1] == '/' || path[end - 1] == '\\')) {
    // Keep the separator of a root: "/" and "C:\".
    if (end == 3 && path[1] == ':') break;
    end--;
  }
  return path.substring(0, end);
}

/// The last component of [location]'s path, or the path itself for a root.
String nameOf(FileDialogLocation location) {
  final trimmed = _trimSeparators(location.path);
  final separator = trimmed.lastIndexOf(RegExp(r'[/\\]'));
  final name = separator < 0 ? trimmed : trimmed.substring(separator + 1);
  return name.isEmpty ? trimmed : name;
}

/// The folders from the top of [location]'s root down to it.
List<FileDialogLocation> ancestry(FileDialogLocation location) {
  final chain = <FileDialogLocation>[location];
  var current = parentOf(location);
  while (current != null) {
    chain.insert(0, current);
    current = parentOf(current);
  }
  return chain;
}
