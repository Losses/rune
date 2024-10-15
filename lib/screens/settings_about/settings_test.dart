import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:rune/widgets/ax_pressure.dart';

import '../../widgets/navigation_bar/navigation_bar_placeholder.dart';
import '../../widgets/playback_controller/controllor_placeholder.dart';

class SettingsTestPage extends StatefulWidget {
  const SettingsTestPage({super.key});

  @override
  State<SettingsTestPage> createState() => _SettingsTestPageState();
}

const globalPadding = EdgeInsets.all(4.0);
const singleSize = 80.0;
const doubleSize = 160.0;

class _SettingsTestPageState extends State<SettingsTestPage> {
  @override
  Widget build(BuildContext context) {
    return Column(children: [
      const NavigationBarPlaceholder(),
      SizedBox(
        width: 320,
        child: TurnstileAnimation(
          tiles: [
            Padding(
              padding: globalPadding,
              child: AxPressure(
                child: Container(
                  width: singleSize,
                  height: singleSize,
                  color: Colors.red,
                ),
              ),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.blue),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.green),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.white),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: doubleSize, height: singleSize, color: Colors.green),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.red),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.blue),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: doubleSize, height: singleSize, color: Colors.blue),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.green),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: singleSize, height: singleSize, color: Colors.white),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: doubleSize, height: singleSize, color: Colors.red),
            ),
            Padding(
              padding: globalPadding,
              child: Container(
                  width: doubleSize, height: singleSize, color: Colors.blue),
            ),
            // Add more tiles as needed
          ],
          enterMode: EnterMode.enter,
          yDirection: YDirection.bottomToTop,
          zDirection: ZDirection.frontToBack,
          duration: const Duration(milliseconds: 1000),
        ),
      ),
      const ControllerPlaceholder(),
    ]);
  }
}

class TurnstileAnimation extends StatefulWidget {
  final List<Widget> tiles;
  final EnterMode enterMode;
  final YDirection yDirection;
  final ZDirection zDirection;
  final Duration duration;

  const TurnstileAnimation({
    super.key,
    required this.tiles,
    this.enterMode = EnterMode.enter,
    this.yDirection = YDirection.bottomToTop,
    this.zDirection = ZDirection.frontToBack,
    this.duration = const Duration(milliseconds: 600),
  });

  @override
  TurnstileAnimationState createState() => TurnstileAnimationState();
}

class TurnstileAnimationState extends State<TurnstileAnimation>
    with SingleTickerProviderStateMixin {
  late AnimationController _controller;
  final List<Animation<double>> _rotationAnimations = [];
  final List<Animation<double>> _opacityAnimations = [];
  final Random _random = Random();

  late List<GlobalKey> _tileKeys;
  final GlobalKey _wrapKey = GlobalKey();
  final List<Offset> _tileOffsets = [];
  final List<Size> _tileSizes = [];

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(vsync: this, duration: widget.duration);
    _tileKeys = List.generate(widget.tiles.length, (_) => GlobalKey());
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _calculateTileOffsets();
      _implementAnimations();
      if (_tileOffsets.length == widget.tiles.length) {
        _controller.forward();
      }
    });
  }

  void _implementAnimations() {
    final int tileCount = widget.tiles.length;
    for (int i = 0; i < tileCount; i++) {
      final beginTime =
          _getBeginTimeFactor(_tileOffsets[i].dx, _tileOffsets[i].dy);

      final rotationTween = Tween<double>(
        begin: widget.enterMode == EnterMode.enter
            ? (widget.zDirection == ZDirection.frontToBack ? 90 : -90)
            : 0,
        end: widget.enterMode == EnterMode.enter
            ? 0
            : (widget.zDirection == ZDirection.frontToBack ? 90 : -90),
      );
      final opacityTween = Tween<double>(
        begin: widget.enterMode == EnterMode.enter ? 0 : 1,
        end: widget.enterMode == EnterMode.enter ? 1 : 0,
      );

      _rotationAnimations.add(
        rotationTween.animate(
          CurvedAnimation(
            parent: _controller,
            curve: Interval(beginTime, 1.0, curve: Curves.easeOutQuint),
          ),
        ),
      );

      _opacityAnimations.add(
        opacityTween.animate(
          CurvedAnimation(
            parent: _controller,
            curve: Interval(beginTime, 1.0, curve: Curves.easeOutQuint),
          ),
        ),
      );
    }
  }

  void _calculateTileOffsets() {
    final RenderBox wrapRenderBox =
        _wrapKey.currentContext!.findRenderObject() as RenderBox;
    final wrapOffset = wrapRenderBox.localToGlobal(Offset.zero);

    for (var key in _tileKeys) {
      final RenderBox tileRenderBox =
          key.currentContext!.findRenderObject() as RenderBox;
      final tileOffset = tileRenderBox.localToGlobal(Offset.zero);
      _tileOffsets.add(tileOffset - wrapOffset);
      _tileSizes.add(tileRenderBox.size);
    }
  }

  double _getBeginTimeFactor(double x, double y) {
    const double xFactor = 4.7143E-4;
    const double yFactor = 0.001714;
    // const double randomFactor = 0.0714;
    const double randomFactor = 0.1;

    final columnFactor =
        widget.enterMode == EnterMode.enter ? xFactor : -xFactor;
    return y * yFactor + x * columnFactor + _random.nextDouble() * randomFactor;
  }

  @override
  Widget build(BuildContext context) {
    return Wrap(
      key: _wrapKey,
      children: List.generate(
        widget.tiles.length,
        (index) => AnimatedBuilder(
          key: _tileKeys[index],
          animation: _controller,
          builder: (context, child) {
            return Transform(
              transform: Matrix4.identity()
                ..setEntry(3, 2, 0.001)
                ..rotateY(
                  _rotationAnimations.isEmpty
                      ? 0
                      : _rotationAnimations[index].value * pi / 180,
                ),
              alignment: _tileOffsets.isEmpty
                  ? null
                  : Alignment(
                      -1 - _tileOffsets[index].dx / _tileSizes[index].width,
                      0.5,
                    ),
              child: Opacity(
                opacity: _opacityAnimations.isEmpty
                    ? 0
                    : _opacityAnimations[index].value,
                child: child,
              ),
            );
          },
          child: widget.tiles[index],
        ),
      ),
    );
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }
}

enum EnterMode { enter, exit }

enum YDirection { topToBottom, bottomToTop }

enum ZDirection { frontToBack, backToFront }
