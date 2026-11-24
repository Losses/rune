import 'package:fluent_ui/fluent_ui.dart';

class InteractiveTag extends BaseButton {
  /// Creates a button.
  const InteractiveTag({
    super.key,
    required super.child,
    required super.onPressed,
    super.onLongPress,
    super.onTapDown,
    super.onTapUp,
    super.focusNode,
    super.autofocus = false,
    super.style,
    super.focusable = true,
  });

  @override
  ButtonStyle defaultStyleOf(BuildContext context) {
    assert(debugCheckHasFluentTheme(context));
    final theme = FluentTheme.of(context);
    return ButtonStyle(
      shadowColor: WidgetStatePropertyAll(theme.shadowColor),
      padding: const WidgetStatePropertyAll(kDefaultButtonPadding),
      shape: WidgetStatePropertyAll(
        RoundedRectangleBorder(borderRadius: BorderRadius.circular(4.0)),
      ),
      backgroundColor: WidgetStateProperty.resolveWith((states) {
        return ButtonThemeData.buttonColor(context, states);
      }),
      foregroundColor: WidgetStateProperty.resolveWith((states) {
        return ButtonThemeData.buttonForegroundColor(context, states);
      }),
    );
  }

  @override
  ButtonStyle? themeStyleOf(BuildContext context) {
    final typography = FluentTheme.of(context).typography;
    assert(debugCheckHasFluentTheme(context));

    return ButtonStyle(textStyle: WidgetStateProperty.all(typography.caption));
  }
}
