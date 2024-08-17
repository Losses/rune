import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/logger.dart';

class BoundingBox {
  final BuildContext context;
  final Offset position;
  final Size size;

  BoundingBox({
    required this.context,
    required this.position,
    required this.size,
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
  final Map<String, BoundingBox> _cachedBoundingBox = {};
  final List<OverlayEntry> _overlayEntries = [];

  void registerKey(String key, GlobalKey globalKey) {
    _registeredKeys[key] = globalKey;

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheBoundingBoxWithKey(key);
    });
  }

  void unregisterKey(String key) {
    _registeredKeys.remove(key);
  }

  void cacheBoundingBoxWithKey(String key) {
    if (_registeredKeys.containsKey(key)) {
      final globalKey = _registeredKeys[key];

      final boundingBox = getBoundingBox(key, globalKey!);

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

  flipAnimation(String fromKey, String toKey) {
    cacheBoundingBoxWithKey(fromKey);
    cacheBoundingBoxWithKey(toKey);

    WidgetsBinding.instance.addPostFrameCallback((_) {
      cacheBoundingBoxWithKey(fromKey);
      cacheBoundingBoxWithKey(toKey);

      final fromBoundingBox = _cachedBoundingBox[fromKey];
      final toBoundingBox = _cachedBoundingBox[toKey];

      if (fromBoundingBox == null) {
        logger.w("Bounding box not found: $fromKey");
        return;
      }
      if (toBoundingBox == null) {
        logger.w("Bounding box not found: $toKey");
        return;
      }

      final mountedContext = fromBoundingBox.context.mounted
          ? fromBoundingBox.context
          : toBoundingBox.context;
      final textWidget = mountedContext.widget as Text?;

      // Create a text overlay in the animation layer and perform a smooth transition animation
      final overlayEntry = OverlayEntry(
        builder: (context) => FlipTextAnimation(
          fromBoundingBox: fromBoundingBox,
          toBoundingBox: toBoundingBox,
          text: textWidget?.data ?? '',
        ),
      );

      _overlayEntries.add(overlayEntry);

      Overlay.of(context, rootOverlay: true).insert(overlayEntry);

      (overlayEntry.builder(context) as FlipTextAnimation)
          .createState()
          .startAnimation();

      overlayEntry.remove();
      _overlayEntries.remove(overlayEntry);
    });
  }

  BoundingBox? getBoundingBox(String key, GlobalKey globalKey) {
    final context = globalKey.currentContext;

    if (context == null) {
      logger.w("Context not found: $key");
      return null;
    }

    final box = context.findRenderObject() as RenderBox;
    final position = box.localToGlobal(Offset.zero);
    final size = box.size;

    return BoundingBox(
      context: context,
      position: position,
      size: size,
    );
  }

  @override
  Widget build(BuildContext context) {
    return widget.child;
  }
}

class FlipText extends StatefulWidget {
  final String flipKey;
  final String text;

  const FlipText({super.key, required this.flipKey, required this.text});

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
    return Text(key: _globalKey, widget.text);
  }
}

class FlipTextAnimation extends StatefulWidget {
  final BoundingBox fromBoundingBox;
  final BoundingBox toBoundingBox;
  final String text;

  const FlipTextAnimation({
    super.key,
    required this.fromBoundingBox,
    required this.toBoundingBox,
    required this.text,
  });

  @override
  FlipTextAnimationState createState() => FlipTextAnimationState();
}

class FlipTextAnimationState extends State<FlipTextAnimation>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  late Animation<Offset> _positionAnimation;

  @override
  void initState() {
    super.initState();

    _controller = AnimationController(
      duration: const Duration(seconds: 1),
      vsync: this,
    );

    _positionAnimation = Tween<Offset>(
      begin: widget.fromBoundingBox.position,
      end: widget.toBoundingBox.position,
    ).animate(CurvedAnimation(
      parent: _controller,
      curve: Curves.easeInOut,
    ));

    _controller.forward();
  }

  Future<void> startAnimation() {
    return _controller.forward();
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, child) {
        return Positioned(
          left: _positionAnimation.value.dx,
          top: _positionAnimation.value.dy,
          child: SizedBox(
            child: Text(widget.text),
          ),
        );
      },
    );
  }
}
