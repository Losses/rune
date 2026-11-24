import 'package:fluent_ui/fluent_ui.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../theme.dart';
import '../../utils/blink_cursor_option.dart';
import '../../widgets/visualizer/visualizer_dark_theme.dart';

void main() {
  test('the argument or the variable launches blinking cursors', () {
    expect(cursorBlinksAtLaunch(const [], const {}), isFalse);
    expect(cursorBlinksAtLaunch(const ['--blink-cursor'], const {}), isTrue);
    expect(
      cursorBlinksAtLaunch(
        const ['--profile', 'a'],
        const {'RUNE_BLINK_CURSOR': '1'},
      ),
      isTrue,
    );
    expect(
      cursorBlinksAtLaunch(const [], const {'RUNE_BLINK_CURSOR': '0'}),
      isFalse,
    );
  });

  test('the app theme fades the cursor unless launched to blink it', () {
    final appTheme = AppTheme();
    for (final brightness in Brightness.values) {
      expect(appTheme.themeData(brightness).cursorOpacityAnimates, isTrue);
    }

    appTheme.cursorBlinks = true;
    for (final brightness in Brightness.values) {
      expect(appTheme.themeData(brightness).cursorOpacityAnimates, isFalse);
    }
  });

  test('the dark variant over scenes keeps the blinking cursor', () {
    final theme = (AppTheme()..cursorBlinks = true).themeData(Brightness.light);
    final dark = darkVariantOf(theme);

    expect(dark.brightness, Brightness.dark);
    expect(dark.cursorOpacityAnimates, isFalse);
    expect(dark.accentColor, theme.accentColor);
  });
}
