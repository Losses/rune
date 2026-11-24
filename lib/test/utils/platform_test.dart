import 'package:flutter_test/flutter_test.dart';
import 'package:rune/utils/platform.dart';

void main() {
  test('tells iPhones, iPads and iOS apps on a Mac apart', () {
    expect(iosDeviceOf(model: 'iPhone', onMac: false), IosDevice.iPhone);
    expect(iosDeviceOf(model: 'iPod touch', onMac: false), IosDevice.iPhone);
    expect(iosDeviceOf(model: 'iPad', onMac: false), IosDevice.iPad);
    expect(iosDeviceOf(model: 'iPad', onMac: true), IosDevice.mac);
    expect(iosDeviceOf(model: 'iPhone', onMac: true), IosDevice.mac);
  });
}
