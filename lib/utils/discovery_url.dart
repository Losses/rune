String _encodeIPv4(String ip) {
  final parts = ip.split('.');
  if (parts.length != 4) throw FormatException('Invalid IPv4');

  final nums = parts.map((p) => int.parse(p)).toList();
  if (nums.any((n) => n < 0 || n > 255)) throw FormatException('Invalid octet');

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

  final encoded = url.substring(7);
  if (encoded.length % 7 != 0) {
    throw FormatException('Invalid encoded IP sequence');
  }

  return List.generate(
    encoded.length ~/ 7,
    (i) => _decodeIPv4(encoded.substring(i * 7, (i + 1) * 7)),
  );
}
