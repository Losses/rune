import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../providers/app_scale.dart';

/// A recovery action available even when zoom makes settings hard to reach.
class AppScaleShortcuts extends StatelessWidget {
  const AppScaleShortcuts({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final macOS = defaultTargetPlatform == TargetPlatform.macOS;
    return Shortcuts(
      shortcuts: <ShortcutActivator, Intent>{
        SingleActivator(
          LogicalKeyboardKey.digit0,
          meta: macOS,
          control: !macOS,
        ): const _ResetScaleIntent(),
      },
      child: Actions(
        actions: <Type, Action<Intent>>{
          _ResetScaleIntent: CallbackAction<_ResetScaleIntent>(
            onInvoke: (_) {
              context
                  .read<AppScaleProvider>()
                  .setPercentage(100)
                  .catchError(
                    (Object error) =>
                        debugPrint('Could not reset interface scale: $error'),
                  );
              return null;
            },
          ),
        },
        child: child,
      ),
    );
  }
}

class _ResetScaleIntent extends Intent {
  const _ResetScaleIntent();
}
