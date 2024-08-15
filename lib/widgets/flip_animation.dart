import 'package:fluent_ui/fluent_ui.dart';

import '../../utils/logger.dart';

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
  final List<OverlayEntry> _overlayEntries = [];

  void registerKey(String key, GlobalKey globalKey) {
    logger.i('Registering flip item: $key');
    _registeredKeys[key] = globalKey;
  }

  void unregisterKey(String key) {
    _registeredKeys.remove(key);
  }

  Future<void> flipAnimation(String fromKey, String toKey) async {
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      // Check if both keys are registered in the container
      if (!_registeredKeys.containsKey(fromKey) ||
          !_registeredKeys.containsKey(toKey)) {
        return;
      }

      final fromGlobalKey = _registeredKeys[fromKey]!;
      final toGlobalKey = _registeredKeys[toKey]!;

      // Get the positions of the two elements relative to the Container
      final fromContext = fromGlobalKey.currentContext;
      final toContext = toGlobalKey.currentContext;

      if (fromContext == null || toContext == null) {
        return;
      }

      final fromBox = fromContext.findRenderObject() as RenderBox;
      final toBox = toContext.findRenderObject() as RenderBox;

      final fromPosition = fromBox.localToGlobal(Offset.zero);
      final toPosition = toBox.localToGlobal(Offset.zero);

      final fromSize = fromBox.size;
      final toSize = toBox.size;

      // Create a text overlay in the animation layer and perform a smooth transition animation
      final overlayEntry = OverlayEntry(
        builder: (context) => FlipTextAnimation(
          fromPosition: fromPosition,
          toPosition: toPosition,
          fromSize: fromSize,
          toSize: toSize,
          text: (fromContext.widget as Text).data ?? '',
        ),
      );

      _overlayEntries.add(overlayEntry);

      Overlay.of(context, rootOverlay: true).insert(overlayEntry);

      await Future.delayed(
          const Duration(seconds: 1)); // Duration of the animation

      overlayEntry.remove();
      _overlayEntries.remove(overlayEntry);
    });
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
    }

    _flipAnimation?.registerKey(widget.flipKey, _globalKey);
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
  final Offset fromPosition;
  final Offset toPosition;
  final Size fromSize;
  final Size toSize;
  final String text;

  const FlipTextAnimation({
    super.key,
    required this.fromPosition,
    required this.toPosition,
    required this.fromSize,
    required this.toSize,
    required this.text,
  });

  @override
  FlipTextAnimationState createState() => FlipTextAnimationState();
}

class FlipTextAnimationState extends State<FlipTextAnimation>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  late Animation<Offset> _positionAnimation;
  late Animation<Size> _sizeAnimation;

  @override
  void initState() {
    super.initState();

    _controller = AnimationController(
      duration: const Duration(seconds: 1),
      vsync: this,
    );

    _positionAnimation = Tween<Offset>(
      begin: widget.fromPosition,
      end: widget.toPosition,
    ).animate(CurvedAnimation(
      parent: _controller,
      curve: Curves.easeInOut,
    ));

    _sizeAnimation = Tween<Size>(
      begin: widget.fromSize,
      end: widget.toSize,
    ).animate(CurvedAnimation(
      parent: _controller,
      curve: Curves.easeInOut,
    ));

    _controller.forward();
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
            width: _sizeAnimation.value.width,
            height: _sizeAnimation.value.height,
            child: Text(widget.text),
          ),
        );
      },
    );
  }
}
