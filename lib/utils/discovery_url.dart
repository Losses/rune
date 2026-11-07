final _ipv4Pattern = RegExp(r'^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$');

/// Returns the four octets of a dotted IPv4 literal, or null for anything else.
List<int>? _ipv4Octets(String ip) {
  final match = _ipv4Pattern.firstMatch(ip);
  if (match == null) return null;

  final octets = [for (var i = 1; i <= 4; i++) int.parse(match.group(i)!)];
  return octets.any((n) => n > 255) ? null : octets;
}

/// Returns the trimmed [input] in canonical dotted form if it is an IPv4
/// literal, otherwise null.
///
/// The canonical form drops leading zeros (`192.168.01.20` becomes
/// `192.168.1.20`). The trust and the library path are keyed by the host, and
/// the path only holds the numeric value, so every step has to use this form.
String? parseIpv4Host(String input) => _ipv4Octets(input.trim())?.join('.');

String _encodeIPv4(String ip) {
  final nums = _ipv4Octets(ip);
  if (nums == null) throw FormatException('Invalid IPv4');

  int num = nums[0] << 24 | nums[1] << 16 | nums[2] << 8 | nums[3];
  final buffer = StringBuffer();

  for (int i = 0; i < 7; i++) {
    final rem = num % 36;
    buffer.writeCharCode(rem < 10 ? 0x30 + rem : 0x61 + rem - 10);
    num = num ~/ 36;
  }

  return buffer.toString().split('').reversed.join();
}

String _decodeIPv4(String s) {
  if (s.length != 7) throw FormatException('Invalid length');

  int num = 0;
  for (final c in s.split('')) {
    final code = c.toLowerCase().codeUnitAt(0);
    final d = code >= 0x30 && code <= 0x39
        ? code - 0x30
        : code >= 0x61 && code <= 0x7a
            ? code - 0x61 + 10
            : throw FormatException('Invalid character');
    num = num * 36 + d;
    if (num > 0xFFFFFFFF) throw FormatException('Overflow');
  }

  final octets = [
    (num >> 24) & 0xFF,
    (num >> 16) & 0xFF,
    (num >> 8) & 0xFF,
    num & 0xFF,
  ];
  return octets.join('.');
}

String encodeRnSrvUrl(List<String> ips) {
  final buffer = StringBuffer('rnsrv://');
  for (final ip in ips) {
    buffer.write(_encodeIPv4(ip));
  }
  return buffer.toString();
}

List<String> decodeRnSrvUrl(String url) {
  if (!url.startsWith('rnsrv://')) {
    throw FormatException('Invalid runep URL');
  }

  final encoded = url.substring('rnsrv://'.length);
  if (encoded.length % 7 != 0) {
    throw FormatException('Invalid encoded IP sequence');
  }

  return List.generate(
    encoded.length ~/ 7,
    (i) => _decodeIPv4(encoded.substring(i * 7, (i + 1) * 7)),
  );
}

/// The library path under which a remote device is stored and opened.
String remoteLibraryPath(List<String> hosts) => '@RR|${encodeRnSrvUrl(hosts)}';
