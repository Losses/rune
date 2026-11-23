import 'package:flutter_test/flutter_test.dart';
import 'package:rune/utils/dialogs/file_dialog/file_dialog_model.dart';

void main() {
  group('the view slider', () {
    test('puts the named views at whole steps', () {
      for (final (index, view) in FileDialogView.named.indexed) {
        expect(view.position, index.toDouble(), reason: '$view');
        expect(FileDialogView.at(index.toDouble()), view);
      }
    });

    test('sizes icons smoothly between Extra large and Medium icons', () {
      final between = FileDialogView.at(0.5);
      expect(between.layout, FileDialogLayout.icons);
      expect(between.iconSize, closeTo(156.8, 0.1));
      expect(between.position, closeTo(0.5, 1e-9));
      expect(FileDialogView.icons(150).position, closeTo(0.545, 0.001));
      // Below Medium icons it snaps to the nearest view.
      expect(FileDialogView.at(2.4), FileDialogView.mediumIcons);
      expect(FileDialogView.at(2.6), FileDialogView.smallIcons);
      expect(FileDialogView.at(6.7), FileDialogView.content);
      expect(FileDialogView.at(-3), FileDialogView.extraLargeIcons);
      expect(FileDialogView.at(12), FileDialogView.content);
    });

    test('keeps icon sizes between Medium and Extra large icons', () {
      expect(FileDialogView.icons(20), FileDialogView.mediumIcons);
      expect(FileDialogView.icons(999), FileDialogView.extraLargeIcons);
    });
  });

  test('the view button steps as Windows does', () {
    var view = FileDialogView.list;
    final seen = <FileDialogView>[];
    for (var press = 0; press < 5; press++) {
      view = view.next;
      seen.add(view);
    }
    expect(seen, [
      FileDialogView.details,
      FileDialogView.tiles,
      FileDialogView.content,
      FileDialogView.largeIcons,
      FileDialogView.list,
    ]);
    // A view outside the cycle steps to its start.
    expect(FileDialogView.mediumIcons.next, FileDialogView.list);
  });

  group('Ctrl and the mouse wheel', () {
    test('size icons by a fraction of a step a notch', () {
      final smaller = FileDialogView.largeIcons.zoomed(1);
      expect(smaller.layout, FileDialogLayout.icons);
      expect(smaller.iconSize, lessThan(96));
      expect(smaller.iconSize, greaterThan(48));
      final larger = FileDialogView.largeIcons.zoomed(-1);
      expect(larger.iconSize, greaterThan(96));
      expect(larger.iconSize, lessThan(256));
    });

    test('stop at Medium icons, then take whole steps', () {
      expect(FileDialogView.icons(50).zoomed(1), FileDialogView.mediumIcons);
      // Less than a whole notch below Medium icons changes nothing.
      expect(
        FileDialogView.mediumIcons.zoomed(0.5),
        FileDialogView.mediumIcons,
      );
      expect(FileDialogView.mediumIcons.zoomed(1), FileDialogView.smallIcons);
      expect(FileDialogView.smallIcons.zoomed(1), FileDialogView.list);
      expect(FileDialogView.details.zoomed(1), FileDialogView.tiles);
      expect(FileDialogView.content.zoomed(1), FileDialogView.content);
      expect(FileDialogView.smallIcons.zoomed(-1), FileDialogView.mediumIcons);
      expect(
        FileDialogView.extraLargeIcons.zoomed(-1),
        FileDialogView.extraLargeIcons,
      );
    });
  });

  test('settings keep the view', () {
    for (final view in [...FileDialogView.named, FileDialogView.icons(150)]) {
      expect(FileDialogView.fromStorageName(view.storageName), view);
    }
    expect(FileDialogView.fromStorageName('icons'), isNull);
    expect(FileDialogView.fromStorageName('sideways'), isNull);
    expect(FileDialogView.fromStorageName(null), isNull);
  });
}
