import 'package:fluent_ui/fluent_ui.dart';

/// The size a [Text] with [style] takes for [text] in [context], optionally
/// wrapped at [maxWidth]. It reads the same inherited settings as [Text], so
/// the measured size matches the drawn one.
Size measureTitle(
  BuildContext context,
  String text,
  TextStyle? style, {
  double maxWidth = double.infinity,
}) {
  final defaultStyle = DefaultTextStyle.of(context);
  var effective = defaultStyle.style.merge(style);
  if (MediaQuery.boldTextOf(context)) {
    effective = effective.merge(const TextStyle(fontWeight: FontWeight.bold));
  }
  final painter = TextPainter(
    text: TextSpan(text: text, style: effective),
    textScaler: MediaQuery.maybeTextScalerOf(context) ?? TextScaler.noScaling,
    textDirection: Directionality.of(context),
    locale: Localizations.maybeLocaleOf(context),
    textWidthBasis: defaultStyle.textWidthBasis,
    textHeightBehavior: defaultStyle.textHeightBehavior,
  )..layout(maxWidth: maxWidth);
  final size = painter.size;
  painter.dispose();
  return size;
}
