import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';

import '../widgets/reveal_effect_context.dart';

import 'reveal_config.dart';
import 'animation_unit.dart';

class RevealEffectController extends ChangeNotifier {
  final RevealConfig config;
  late final AnimationUnit _animationUnit;
  late final RevealEffectContextState _effectContext;
  final GlobalKey _widgetKey = GlobalKey();
  Offset? _localPosition;

  AnimationUnit get animationUnit => _animationUnit;
  bool get mouseReleased => _animationUnit.mouseReleased;
  double get mouseDownAnimateLogicFrame =>
      _animationUnit.mouseDownAnimateLogicFrame;
  Offset? get localPosition => _localPosition;

  RevealEffectController(BuildContext context, this.config) {
    _effectContext = RevealEffectContext.of(context);

    _animationUnit = AnimationUnit(controller: this);
    _effectContext.addUnit(_animationUnit);
  }

  void updateMousePosition(Offset? position) {
    if (position != null && _widgetKey.currentContext != null) {
      final RenderBox renderBox =
          _widgetKey.currentContext!.findRenderObject() as RenderBox;
      final Offset localPosition = renderBox.globalToLocal(position);

      final w = renderBox.size.width;
      final h = renderBox.size.height;
      final s = renderBox.size.shortestSide;

      final hR = s * config.hoverLightFillRadius;
      final bR = s * config.borderFillRadius;

      final r = max(hR, bR);

      // Check if the point is within the widget's bounds
      if (localPosition.dx >= 0 - r &&
          localPosition.dy >= 0 - r &&
          localPosition.dx <= w + r &&
          localPosition.dy <= h + r) {
        _localPosition = localPosition;
      } else {
        _localPosition = null;
      }
    } else {
      _localPosition = null;
    }

    notify();
  }

  void mouseDown() {
    _animationUnit.mouseReleased = false;
    _effectContext.startAnimation(_animationUnit);
  }

  void mouseUp() {
    _animationUnit.mouseReleased = true;
    _animationUnit.mouseDownAnimateReleasedFrame =
        _animationUnit.mouseDownAnimateCurrentFrame;
  }

  void mouseExit() {
    _localPosition = null;
    notify();
  }

  @override
  dispose() {
    super.dispose();
    _effectContext.removeUnit(_animationUnit);
  }

  notify() {
    notifyListeners();
  }

  GlobalKey get widgetKey => _widgetKey;
}
