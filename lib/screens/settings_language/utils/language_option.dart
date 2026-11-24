import 'package:fluent_ui/fluent_ui.dart';

class LanguageOption {
  final String title;
  final String sampleText;
  final Locale locale;
  final bool experimental;

  const LanguageOption({
    required this.title,
    required this.sampleText,
    required this.locale,
    this.experimental = false,
  });
}
