import 'package:fluent_ui/fluent_ui.dart';
import 'package:go_router/go_router.dart';
import 'package:material_symbols_icons/symbols.dart';

class CoverWallButton extends StatelessWidget {
  const CoverWallButton({super.key});

  @override
  Widget build(BuildContext context) {
    return IconButton(
      onPressed: () {
        final routeState = GoRouterState.of(context);

        if (routeState.fullPath == "/cover_wall") {
          if (context.canPop()) {
            context.pop();
          }
        } else {
          context.push("/cover_wall");
        }
      },
      icon: const Icon(Symbols.photo_frame),
    );
  }
}
