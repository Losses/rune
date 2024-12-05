import 'dart:ui';

Locale? localeFromString(String? x) {
  if (x == null) return null;

  final parts = x.split('|');
  String? languageCode;
  String? scriptCode;
  String? countryCode;

  for (var part in parts) {
    if (part.startsWith('s_')) {
      scriptCode = part.substring(2);
    } else if (part.startsWith('c_')) {
      countryCode = part.substring(2);
    } else {
      languageCode ??= part;
    }
  }

  if (languageCode != null) {
    return Locale.fromSubtags(
      languageCode: languageCode,
      scriptCode: scriptCode,
      countryCode: countryCode,
    );
  }

  return null;
}
