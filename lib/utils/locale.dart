import 'dart:ui';

Locale? localeFromString(String? x) {
  if (x == null) return null;

  final parts = x.split('_');
  if (parts.isNotEmpty) {
    if (parts.length == 3) {
      return Locale.fromSubtags(
        languageCode: parts[0],
        scriptCode: parts[1],
        countryCode: parts[2],
      );
    } else if (parts.length == 2) {
      return Locale.fromSubtags(
        languageCode: parts[0],
        countryCode: parts[1],
      );
    } else if (parts.length == 1) {
      return Locale(parts[0]);
    }
  }

  return null;
}
