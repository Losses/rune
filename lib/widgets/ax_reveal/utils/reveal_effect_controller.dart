import 'package:fluent_ui/fluent_ui.dart';

import '../widgets/reveal_effect_context.dart';

import 'animation_unit.dart';

class RevealEffectController extends ChangeNotifier {
  late final AnimationUnit _animationUnit;
  late final RevealEffectContextState _effectContext;
  final GlobalKey _widgetKey = GlobalKey();
  Offset? _localPosition;

  AnimationUnit get animationUnit => _animationUnit;
  bool get mouseReleased => _animationUnit.mouseReleased;
  double get mouseDownAnimateLogicFrame =>
      _animationUnit.mouseDownAnimateLogicFrame;
  Offset? get localPosition => _localPosition;

  RevealEffectController(BuildContext context) {
    _effectContext = RevealEffectContext.of(context);

    _animationUnit = AnimationUnit(controller: this);
    _effectContext.addUnit(_animationUnit);
  }

  void updateMousePosition(Offset? position) {
    if (position != null && _widgetKey.currentContext != null) {
      final RenderBox renderBox =
          _widgetKey.currentContext!.findRenderObject() as RenderBox;
      final Offset localPosition = renderBox.globalToLocal(position);

      _localPosition = localPosition;
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
