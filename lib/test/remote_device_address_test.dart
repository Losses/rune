import 'package:flutter_test/flutter_test.dart';

import '../screens/settings_neighbors/utils/remote_device_address.dart';
import '../utils/discovery_url.dart';

void main() {
  group('parseIpv4Host', () {
    test('accepts IPv4 literals and trims whitespace', () {
      expect(parseIpv4Host('192.168.1.20'), '192.168.1.20');
      expect(parseIpv4Host('  10.0.0.1 '), '10.0.0.1');
      expect(parseIpv4Host('0.0.0.0'), '0.0.0.0');
      expect(parseIpv4Host('255.255.255.255'), '255.255.255.255');
    });

    test('rejects everything else', () {
      for (final input in [
        '',
        'localhost',
        'example.com',
        '192.168.1',
        '192.168.1.1.1',
        '256.1.1.1',
        '1.1.1.-1',
        '+1.1.1.1',
        '1.1.1.1:7863',
        'ws://1.1.1.1',
        '::1',
        '1..1.1',
      ]) {
        expect(parseIpv4Host(input), isNull, reason: input);
      }
    });
  });

  group('encodeRnSrvUrl', () {
    test('refuses anything but IPv4 literals', () {
      for (final input in ['localhost', '256.1.1.1', '+1.1.1.1', ' 1.1.1.1']) {
        expect(() => encodeRnSrvUrl([input]), throwsFormatException,
            reason: input);
      }
    });
  });

  group('remoteLibraryPath', () {
    test('wraps the rnsrv url in the remote prefix', () {
      final path = remoteLibraryPath(['192.168.1.20']);
      expect(path, startsWith('@RR|rnsrv://'));
      expect(path, '@RR|${encodeRnSrvUrl(['192.168.1.20'])}');
      expect(decodeRnSrvUrl(path.substring('@RR|'.length)), ['192.168.1.20']);
    });
  });
}
