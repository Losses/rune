import 'package:flutter_test/flutter_test.dart';

import 'package:rune/utils/discovery_url.dart';

void main() {
  test('round-trips one and several IPv4 addresses', () {
    for (final ips in [
      ['192.168.1.5'],
      ['10.0.0.1', '255.255.255.255', '0.0.0.0'],
    ]) {
      expect(decodeRnSrvUrl(encodeRnSrvUrl(ips)), ips);
    }
  });

  test('rejects a URL with the wrong scheme', () {
    expect(() => decodeRnSrvUrl('rnsrv:/abc'), throwsFormatException);
  });
}
