import 'package:flutter_test/flutter_test.dart';
import 'package:rune/screens/settings_about/settings_about.dart';

void main() {
  test('shows at most eight characters of the build hash', () {
    expect(shortBuildHash('3768adc3607ecc01e431'), '3768adc3');
    expect(shortBuildHash('3768adc'), '3768adc');
    expect(shortBuildHash(''), '');
  });
}
