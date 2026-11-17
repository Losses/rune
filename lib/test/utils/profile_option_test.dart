import 'package:flutter_test/flutter_test.dart';

import '../../utils/profile_option.dart';

void main() {
  test('a trailing profile argument falls back to the default profile', () {
    expect(profileAtLaunch(const ['--profile']), isNull);
  });

  test('a valid value is preserved and no option selects the default', () {
    expect(profileAtLaunch(const []), isNull);
    expect(profileAtLaunch(const ['--profile', 'work']), 'work');
    expect(
      profileAtLaunch(const ['--profile', 'work', '--blink-cursor']),
      'work',
    );
  });
}
