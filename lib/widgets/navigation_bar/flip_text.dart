import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/logger.dart';
import './flip_animation_manager.dart';

class FlipText extends StatefulWidget {
  final String flipKey;
  final String text;
  final double scale;
  final double? alpha;
  final double? fontWeight;
  final bool hidden;
  final Color? color;
  final Color glowColor;
  final double glowRadius;

  const FlipText({
    super.key,
    required this.flipKey,
    required this.text,
    this.hidden = false,
    this.scale = 1,
    this.alpha,
    this.fontWeight,
    this.color,
    this.glowColor = Colors.transparent,
    this.glowRadius = 0,
  });

  @override
  FlipTextState createState() => FlipTextState();
}

class FlipTextState extends State<FlipText> {
  final GlobalKey _globalKey = GlobalKey();
  FlipAnimationManagerState? _flipAnimation;
  bool _visible = true;

  registerKey() {
    if (_flipAnimation == null) {
      logger.w("Flip context not found for ${widget.flipKey}");
      return;
    } else {
      _flipAnimation!.registerKey(widget.flipKey, _globalKey);
    }
  }

  void setVisibility(bool visible) {
    setState(() {
      _visible = visible;
    });
  }

  void updateStyle() {
    final theme = FluentTheme.of(context);
    final defaultTextStyle = theme.typography.body;

    _flipAnimation?.registerStyle(
      widget.flipKey,
      widget.scale,
      widget.fontWeight ?? 400,
      widget.color ?? defaultTextStyle!.color!,
      widget.alpha ?? 255,
    );
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _flipAnimation = FlipAnimationManager.of(context);

    registerKey();
    updateStyle();
  }

  @override
  void didUpdateWidget(covariant FlipText oldWidget) {
    super.didUpdateWidget(oldWidget);

    updateStyle();
  }

  @override
  void dispose() {
    _flipAnimation?.unregisterKey(widget.flipKey);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = FluentTheme.of(context);
    final defaultTextStyle = theme.typography.body;
    final initialColor = (widget.color ?? defaultTextStyle!.color)
        ?.withAlpha(widget.alpha?.toInt() ?? 255);

    return Visibility(
      maintainSize: true,
      maintainAnimation: true,
      maintainState: true,
      visible: _visible && !widget.hidden,
      child: TweenAnimationBuilder(
        tween: ColorTween(
          begin: initialColor,
          end: initialColor,
        ),
        duration: theme.fastAnimationDuration,
        builder: (context, Color? color, child) {
          return TweenAnimationBuilder(
            tween: ColorTween(
              begin: widget.glowColor,
              end: widget.glowColor,
            ),
            duration: theme.fastAnimationDuration,
            builder: (context, Color? glowColor, child) {
              return TweenAnimationBuilder(
                tween: Tween<double>(
                  begin: widget.glowRadius,
                  end: widget.glowRadius,
                ),
                duration: theme.fastAnimationDuration,
                builder: (context, double glowRadius, child) {
                  return TweenAnimationBuilder(
                    tween: Tween<double>(
                      begin: widget.alpha?.toDouble() ?? 255.0,
                      end: widget.alpha?.toDouble() ?? 255.0,
                    ),
                    duration: theme.fastAnimationDuration,
                    builder: (context, double alpha, child) {
                      return Transform.scale(
                        key: _globalKey,
                        scale: widget.scale,
                        alignment: Alignment.topLeft,
                        child: Text(
                          widget.text,
                          style: TextStyle(
                            fontSize: 17,
                            fontVariations: <FontVariation>[
                              FontVariation('wght', widget.fontWeight ?? 400),
                            ],
                            color: color?.withAlpha(alpha.toInt()),
                            shadows: glowRadius <= 0
                                ? null
                                : [
                                    Shadow(
                                      color: glowColor ?? Colors.transparent,
                                      blurRadius: glowRadius,
                                    )
                                  ],
                          ),
                        ),
                      );
                    },
                  );
                },
              );
            },
          );
        },
      ),
    );
  }
}
