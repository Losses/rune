import 'package:go_router/go_router.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:fluent_ui/fluent_ui.dart' hide Page;

import '../../utils/navigation/utils/navigation_backward.dart';

import '../../widgets/ax_pressure.dart';
import '../../widgets/hover_opacity.dart';

class BackButton extends StatefulWidget {
  const BackButton({
    super.key,
  });

  @override
  State<BackButton> createState() => _BackButtonState();
}

class _BackButtonState extends State<BackButton> {
  final FocusNode _focusNode = FocusNode(debugLabel: 'Back Button');

  @override
  void dispose() {
    _focusNode.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final routerState = GoRouterState.of(context);

    if (routerState.fullPath == '/library') {
      return Container();
    }

    return Positioned(
      top: -12,
      left: -12,
      child: AxPressure(
        child: GestureDetector(
          onTap: () {
            navigateBackwardWithPop(context);
          },
          child: HoverOpacity(
            child: FocusableActionDetector(
              focusNode: _focusNode,
              child: SvgPicture.asset(
                'assets/arrow-circle-left-solid.svg',
                width: 56,
                colorFilter: ColorFilter.mode(
                  FluentTheme.of(context).inactiveColor,
                  BlendMode.srcIn,
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
