import 'package:fluent_ui/fluent_ui.dart';

import '../../config/routes.dart';
import '../../widgets/router/router_animation.dart';

import 'no_effect_page_route.dart';
import 'rune_router_frame_implementation.dart';

@immutable
class RuneWithNavigationBarAndPlaybackControllor extends StatefulWidget {
  static RuneWithNavigationBarAndPlaybackControllorState of(
    BuildContext context,
  ) {
    return context.findAncestorStateOfType<
        RuneWithNavigationBarAndPlaybackControllorState>()!;
  }

  const RuneWithNavigationBarAndPlaybackControllor({
    super.key,
    required this.routeName,
  });

  final String routeName;

  @override
  RuneWithNavigationBarAndPlaybackControllorState createState() =>
      RuneWithNavigationBarAndPlaybackControllorState();
}

final runeWithNavigationBarAndPlaybackControllorNavigatorKey =
    GlobalKey<NavigatorState>();

class RuneWithNavigationBarAndPlaybackControllorState
    extends State<RuneWithNavigationBarAndPlaybackControllor> {
  @override
  void initState() {
    super.initState();
  }

  @override
  Widget build(BuildContext context) {
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, _) async {
        if (didPop) return;
      },
      child: RuneRouterFrameImplementation(
        child: Navigator(
          key: runeWithNavigationBarAndPlaybackControllorNavigatorKey,
          initialRoute: widget.routeName,
          onGenerateRoute: _onGenerateRoute,
        ),
      ),
    );
  }

  Route<Widget> _onGenerateRoute(RouteSettings settings) {
    final routeName = settings.name;
    final builder = routes[routeName];

    return NoEffectPageRoute(
      builder: (context) {
        if (builder == null) {
          return Container();
        }
        return RouterAnimation(builder(context));
      },
      settings: settings,
    );
  }
}
