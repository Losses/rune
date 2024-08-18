import 'dart:async';
import 'package:fluent_ui/fluent_ui.dart';
import 'package:player/widgets/lerp_controller.dart';

import '../../utils/logger.dart';

class TextStyleSheet {
  final BuildContext context;
  final Offset position;
  final double scale;
  final double fontWeight;
  final Color color;

  TextStyleSheet({
    required this.context,
    required this.position,
    required this.scale,
    required this.fontWeight,
    required this.color,
  });
}

class FlipAnimationContext extends StatelessWidget {
  final Widget child;

  const FlipAnimationContext({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    return Stack(
      alignment: Alignment.center,
      children: [
        SizedBox.expand(
          child: FlipAnimationManager(child: child),
        ),
        const Overlay(
          initialEntries: [],
        ),
      ],
    );
  }
}

class FlipAnimationManager extends StatefulWidget {
  final Widget child;

  const FlipAnimationManager({super.key, required this.child});

  static FlipAnimationManagerState? of(BuildContext context) {
    return context.findAncestorStateOfType<FlipAnimationManagerState>();
  }

  @override
  FlipAnimationManagerState createState() => FlipAnimationManagerState();
}

class FlipAnimationManagerState extends State<FlipAnimationManager> {
  final Map<String, GlobalKey> _registeredKeys = {};
  final Map<String, TextStyleSheet> _cachedBoundingBox = {};
  final List<OverlayEntry> _overlayEntries = [];

  void registerKey(String key, GlobalKey globalKey) {
    _registeredKeys[key] = globalKey;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheStyleSheetWithKey(key);
    });
  }

  void unregisterKey(String key) {
    _registeredKeys.remove(key);
  }

  void cacheStyleSheetWithKey(String key) {
    if (_registeredKeys.containsKey(key)) {
      final globalKey = _registeredKeys[key];

      final boundingBox = getStyleSheet(key, globalKey!);

      if (boundingBox != null) {
        _cachedBoundingBox[key] = boundingBox;

        // logger.i("Cached bounding box cached: $key");
      } else {
        // logger.w("Cached bounding not found: $key");
      }
    } else {
      // logger.w("Key not registered: $key");
    }
  }

  Future<bool> flipAnimation(String fromKey, String toKey) async {
    cacheStyleSheetWithKey(fromKey);
    cacheStyleSheetWithKey(toKey);

    final completer = Completer<bool>();

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheStyleSheetWithKey(fromKey);
      cacheStyleSheetWithKey(toKey);

      final fromBoundingBox = _cachedBoundingBox[fromKey];
      final toBoundingBox = _cachedBoundingBox[toKey];

      if (fromBoundingBox == null) {
        completer.complete(false);
        return;
      }
      if (toBoundingBox == null) {
        completer.complete(false);
        return;
      }

      final mountedContext = fromBoundingBox.context.mounted
          ? fromBoundingBox.context
          : toBoundingBox.context;
      final transformWidget = mountedContext.widget as Transform?;
      final textWidget = transformWidget?.child as Text?;

      // Declare the overlayEntry variable first
      late OverlayEntry overlayEntry;

      // Create a text overlay in the animation layer and perform a smooth transition animation
      overlayEntry = OverlayEntry(
        builder: (context) => FlipTextAnimation(
          fromStyles: fromBoundingBox,
          toStyles: toBoundingBox,
          text: textWidget?.data ?? '',
          onAnimationComplete: () {
            overlayEntry.remove();
            _overlayEntries.remove(overlayEntry);
            completer.complete(true);
          },
        ),
      );

      _overlayEntries.add(overlayEntry);

      Overlay.of(context, rootOverlay: true).insert(overlayEntry);

      (overlayEntry.builder(context) as FlipTextAnimation).createState();
    });

    return completer.future;
  }

  double? getFontVariationValue(List<FontVariation>? variations, String name) {
    if (variations == null) return null;

    for (var variation in variations) {
      if (variation.axis == name) {
        return variation.value;
      }
    }
    return null;
  }

  TextStyleSheet? getStyleSheet(String key, GlobalKey globalKey) {
    final context = globalKey.currentContext;

    if (context == null) {
      logger.w("Context not found: $key");
      return null;
    }

    if (!context.mounted) {
      logger.w("Widget not mounted: $key");
      return null;
    }

    final box = context.findRenderObject() as RenderBox;
    final position = box.localToGlobal(Offset.zero);

    final transformWidget = context.widget as Transform;
    final textWidget = transformWidget.child as Text;
    final style = textWidget.style;

    final DefaultTextStyle defaultTextStyle = DefaultTextStyle.of(context);
    TextStyle? effectiveTextStyle = style;
    if (style == null || style.inherit) {
      effectiveTextStyle = defaultTextStyle.style.merge(style);
    }
    if (MediaQuery.boldTextOf(context)) {
      effectiveTextStyle = effectiveTextStyle!
          .merge(const TextStyle(fontWeight: FontWeight.bold));
    }

    return TextStyleSheet(
        context: context,
        position: position,
        scale: transformWidget.transform.row0[0],
        fontWeight:
            getFontVariationValue(effectiveTextStyle?.fontVariations, 'wght') ??
                400,
        color: (effectiveTextStyle?.color)!);
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}

class FlipText extends StatefulWidget {
  final String flipKey;
  final String text;
  final double scale;
  final double? alpha;
  final double? fontWeight;
  final bool hidden;

  const FlipText(
      {super.key,
      required this.flipKey,
      required this.text,
      required this.hidden,
      this.scale = 1,
      this.alpha,
      this.fontWeight});

  @override
  FlipTextState createState() => FlipTextState();
}

class FlipTextState extends State<FlipText> {
  final GlobalKey _globalKey = GlobalKey();
  FlipAnimationManagerState? _flipAnimation;

  registerKey() {
    _flipAnimation = FlipAnimationManager.of(context);

    if (_flipAnimation == null) {
      logger.w("Flip context not found for ${widget.flipKey}");
      return;
    } else {
      _flipAnimation!.registerKey(widget.flipKey, _globalKey);
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    registerKey();
  }

  @override
  void dispose() {
    _flipAnimation?.unregisterKey(widget.flipKey);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final DefaultTextStyle defaultTextStyle = DefaultTextStyle.of(context);

    return Visibility(
      maintainSize: true,
      maintainAnimation: true,
      maintainState: true,
      visible: !widget.hidden,
      child: Transform.scale(
          key: _globalKey,
          scale: widget.scale,
          alignment: Alignment.topLeft,
          child: Text(
            widget.text,
            style: TextStyle(
              fontVariations: <FontVariation>[
                FontVariation('wght', widget.fontWeight ?? 400),
              ],
              color: defaultTextStyle.style.color!
                  .withAlpha(widget.alpha?.toInt() ?? 255),
            ),
          )),
    );
  }
}

class FlipTextAnimation extends StatefulWidget {
  final TextStyleSheet fromStyles;
  final TextStyleSheet toStyles;
  final String text;
  final VoidCallback onAnimationComplete;

  const FlipTextAnimation({
    super.key,
    required this.fromStyles,
    required this.toStyles,
    required this.text,
    required this.onAnimationComplete,
  });

  @override
  FlipTextAnimationState createState() => FlipTextAnimationState();
}

class FlipTextAnimationState extends State<FlipTextAnimation>
    with TickerProviderStateMixin {
  late LerpController _positionXController;
  late LerpController _positionYController;
  late LerpController _scaleController;
  late LerpController _alphaController;
  late LerpController _fontWeightController;

  late double x;
  late double y;
  late double scale;
  late double alpha;
  late double fontWeight;

  @override
  void initState() {
    super.initState();

    x = widget.fromStyles.position.dx;
    y = widget.fromStyles.position.dy;
    scale = widget.fromStyles.scale;
    alpha = widget.fromStyles.color.alpha.toDouble();
    fontWeight = widget.fromStyles.fontWeight;

    _positionXController = LerpController(
      initialValue: x,
      getter: () => x,
      setter: (value) => setState(() {
        x = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _positionYController = LerpController(
      initialValue: y,
      getter: () => y,
      setter: (value) => setState(() {
        y = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _scaleController = LerpController(
      initialValue: scale,
      getter: () => scale,
      setter: (value) => setState(() {
        scale = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _alphaController = LerpController(
      initialValue: alpha,
      getter: () => alpha,
      setter: (value) => setState(() {
        alpha = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _fontWeightController = LerpController(
      initialValue: fontWeight,
      getter: () => fontWeight,
      setter: (value) => setState(() {
        fontWeight = value;
      }),
      t: 0.15,
      vsync: this,
    );

    _startAnimation();
  }

  Future<void> _startAnimation() async {
    List<Future<void>> futures = [
      _positionXController.lerp(widget.toStyles.position.dx),
      _positionYController.lerp(widget.toStyles.position.dy),
      _scaleController.lerp(widget.toStyles.scale),
      _alphaController.lerp(widget.toStyles.color.alpha.toDouble()),
      _fontWeightController.lerp(widget.toStyles.fontWeight),
    ];

    await Future.wait(futures);

    widget.onAnimationComplete();
  }

  @override
  void dispose() {
    _positionXController.dispose();
    _positionYController.dispose();
    _scaleController.dispose();
    _alphaController.dispose();
    _fontWeightController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Positioned(
      left: x,
      top: y,
      child: Transform.scale(
          scale: scale,
          alignment: Alignment.topLeft,
          child: Text(
            widget.text,
            style: TextStyle(
              fontVariations: <FontVariation>[
                FontVariation('wght', fontWeight)
              ],
              color: widget.toStyles.color.withAlpha(alpha.toInt()),
            ),
          )),
    );
  }
}
