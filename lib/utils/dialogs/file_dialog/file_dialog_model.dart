import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../../../bindings/bindings.dart';

/// How the item area lays items out.
enum FileDialogLayout { icons, smallIcons, list, details, tiles, content }

/// What the item area shows: a layout and the edge of its icons. Icons take
/// any size from Medium to Extra large icons, as File Explorer's do when its
/// view slider is dragged or Ctrl and the mouse wheel are used; every other
/// layout has one size.
@immutable
class FileDialogView {
  const FileDialogView._(this.layout, this.iconSize);

  /// Icons of [size], kept between Medium and Extra large icons.
  factory FileDialogView.icons(double size) => FileDialogView._(
    FileDialogLayout.icons,
    size.clamp(mediumIcons.iconSize, extraLargeIcons.iconSize).toDouble(),
  );

  static const extraLargeIcons = FileDialogView._(FileDialogLayout.icons, 256);
  static const largeIcons = FileDialogView._(FileDialogLayout.icons, 96);
  static const mediumIcons = FileDialogView._(FileDialogLayout.icons, 48);
  static const smallIcons = FileDialogView._(FileDialogLayout.smallIcons, 16);
  static const list = FileDialogView._(FileDialogLayout.list, 16);
  static const details = FileDialogView._(FileDialogLayout.details, 16);
  static const tiles = FileDialogView._(FileDialogLayout.tiles, 48);
  static const content = FileDialogView._(FileDialogLayout.content, 32);

  /// File Explorer's eight named views from the largest icons down, as its
  /// view slider lists them and Ctrl+Shift+1 to 8 pick them.
  static const named = [
    extraLargeIcons,
    largeIcons,
    mediumIcons,
    smallIcons,
    list,
    details,
    tiles,
    content,
  ];

  /// The views the view button steps through when pressed, as Windows'
  /// "Change your view" button does.
  static const cycle = [list, details, tiles, content, largeIcons];

  final FileDialogLayout layout;

  /// The edge of an item's icon in logical pixels.
  final double iconSize;

  /// Whether the view is large enough to show thumbnails instead of icons.
  bool get showsThumbnails => iconSize >= 32;

  /// Where the view sits on the view slider: 0 for Extra large icons down
  /// to 7 for Content, in between for the icon sizes between the named
  /// ones, which are spaced by the ratio of their sizes.
  double get position {
    if (layout != FileDialogLayout.icons) return named.indexOf(this).toDouble();
    double between(FileDialogView large, FileDialogView small, int index) =>
        index +
        math.log(large.iconSize / iconSize) /
            math.log(large.iconSize / small.iconSize);
    return iconSize >= largeIcons.iconSize
        ? between(extraLargeIcons, largeIcons, 0)
        : between(largeIcons, mediumIcons, 1);
  }

  /// The view at [position] on the view slider: any icon size above Medium
  /// icons, the nearest named view below.
  factory FileDialogView.at(double position) {
    final clamped = position.clamp(0.0, named.length - 1.0);
    if (clamped > 2) {
      return named[clamped.round()];
    }
    double size(FileDialogView large, FileDialogView small, double fraction) =>
        large.iconSize *
        math.pow(small.iconSize / large.iconSize, fraction).toDouble();
    return FileDialogView.icons(
      clamped <= 1
          ? size(extraLargeIcons, largeIcons, clamped)
          : size(largeIcons, mediumIcons, clamped - 1),
    );
  }

  /// The next view the view button shows.
  FileDialogView get next {
    final index = cycle.indexOf(this);
    return cycle[(index + 1) % cycle.length];
  }

  /// Ctrl and the mouse wheel turned [notches]: down (positive) for smaller
  /// items, up for larger. Each notch sizes icons by [_slideOfANotch] of a
  /// step of the slider, so Extra large to Medium icons takes about a dozen
  /// notches. Going down, Medium icons is a stop; from there every view is a
  /// whole step, taken once a whole notch has turned.
  FileDialogView zoomed(double notches) {
    final from = position;
    final medium = named.indexOf(mediumIcons).toDouble();
    if (from < medium || (from == medium && notches < 0)) {
      final to = from + notches * _slideOfANotch;
      return to > medium ? mediumIcons : FileDialogView.at(to);
    }
    if (notches.abs() < 1) return this;
    return FileDialogView.at(from + notches.sign);
  }

  static const _slideOfANotch = 0.15;

  /// How the view is kept in settings.
  String get storageName => layout == FileDialogLayout.icons
      ? '${layout.name}:${iconSize.round()}'
      : layout.name;

  /// A view kept with [storageName], or null for anything else.
  static FileDialogView? fromStorageName(String? name) {
    if (name == null) return null;
    final [layoutName, ...rest] = name.split(':');
    final layout = FileDialogLayout.values
        .where((value) => value.name == layoutName)
        .firstOrNull;
    if (layout == null) return null;
    if (layout == FileDialogLayout.icons) {
      final size = double.tryParse(rest.firstOrNull ?? '');
      return size == null ? null : FileDialogView.icons(size);
    }
    return named.firstWhere((view) => view.layout == layout);
  }

  @override
  bool operator ==(Object other) =>
      other is FileDialogView &&
      other.layout == layout &&
      other.iconSize == iconSize;

  @override
  int get hashCode => Object.hash(layout, iconSize);

  @override
  String toString() => 'FileDialogView($storageName)';
}

/// The columns of the details layout, which are also what items sort by,
/// with the width each has until it is resized. The name takes the room the
/// others leave until it is resized itself.
enum FileDialogColumn {
  name(260),
  dateModified(160),
  type(130),
  size(96);

  const FileDialogColumn(this.defaultWidth);

  final double defaultWidth;
}

class FileDialogSort {
  const FileDialogSort(this.column, this.ascending);

  final FileDialogColumn column;
  final bool ascending;

  /// Sorting by the same column again flips the order, as column headers do.
  FileDialogSort toggled(FileDialogColumn next) => next == column
      ? FileDialogSort(column, !ascending)
      : FileDialogSort(next, true);
}

/// The extension of [name] without the dot, lowercased, or empty when it has
/// none. A leading dot alone (".profile") marks a hidden name and has no
/// extension.
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
      FileDialogColumn.name => byName(a, b),
      FileDialogColumn.dateModified => (a.modified?.toInt() ?? 0).compareTo(
        b.modified?.toInt() ?? 0,
      ),
      FileDialogColumn.type => extensionOf(
        a.name,
      ).compareTo(extensionOf(b.name)),
      FileDialogColumn.size => a.size.toInt().compareTo(b.size.toInt()),
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
