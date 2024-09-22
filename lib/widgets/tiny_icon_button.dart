import 'package:fluent_ui/fluent_ui.dart';

class TinyIconButton extends BaseButton {
  const TinyIconButton({
    super.key,
    required Widget icon,
    required super.onPressed,
    super.onLongPress,
    super.onTapDown,
    super.onTapUp,
    super.focusNode,
    super.autofocus = false,
    super.style,
    super.focusable = true,
    this.iconButtonMode,
  }) : super(child: icon);

  final IconButtonMode? iconButtonMode;

  @override
  ButtonStyle defaultStyleOf(BuildContext context) {
    assert(debugCheckHasFluentTheme(context));
    final theme = FluentTheme.of(context);
    final isIconSmall = SmallIconButton.of(context) != null ||
        iconButtonMode == IconButtonMode.tiny;
    return ButtonStyle(
      iconSize: WidgetStatePropertyAll(isIconSmall ? 11.0 : null),
      padding: const WidgetStatePropertyAll(EdgeInsets.all(0.0)),
      backgroundColor: WidgetStateProperty.resolveWith((states) {
        return Colors.transparent;
      }),
      foregroundColor: WidgetStateProperty.resolveWith((states) {
        if (states.isDisabled) return theme.resources.textFillColorDisabled;
        if (states.isHovered) return theme.resources.textFillColorPrimary.withAlpha(160);
        return null;
      }),
      shape: WidgetStatePropertyAll(RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(4.0),
      )),
    );
  }

  @override
  ButtonStyle? themeStyleOf(BuildContext context) {
    assert(debugCheckHasFluentTheme(context));
    return ButtonTheme.of(context).iconButtonStyle;
  }
}
