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

  const FlipText(
      {super.key,
      required this.flipKey,
      required this.text,
      this.hidden = false,
      this.scale = 1,
      this.alpha,
      this.fontWeight});

  @override
  FlipTextState createState() => FlipTextState();
}

class FlipTextState extends State<FlipText> {
  final GlobalKey _globalKey = GlobalKey();
  FlipAnimationManagerState? _flipAnimation;
  bool _visible = true;

  registerKey() {
    _flipAnimation = FlipAnimationManager.of(context);

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
      visible: _visible && !widget.hidden,
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
