import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/band_screen_fallback_button.dart';
import '../../providers/responsive_providers.dart';

class RemoveDialogOnBand extends StatelessWidget {
  const RemoveDialogOnBand({
    super.key,
    required this.child,
    required this.onConfirm,
    required this.$close,
    this.icon,
    this.isLoading = false,
    this.error,
  });

  final IconData? icon;
  final Widget child;
  final VoidCallback onConfirm;
  final void Function(Null) $close;

  /// Keeps the route open and disables both compact actions during removal.
  /// Leave Fluent's `dismissWithEsc` override off so Escape consults this scope.
  final bool isLoading;

  /// Localized failure to show beside the compact actions when removal fails.
  final String? error;

  @override
  Widget build(BuildContext context) {
    final dialog = DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.belt,
        DeviceType.tv,
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.belt ||
            activeBreakpoint == DeviceType.dock) {
          return LayoutBuilder(
            builder: (context, constraint) {
              final size = min(constraint.maxWidth, constraint.maxHeight);

              final children = [
                if (error case final message?)
                  Flexible(
                    child: SingleChildScrollView(
                      padding: const EdgeInsets.all(4),
                      child: Text(message, textAlign: TextAlign.center),
                    ),
                  ),
                BandScreenFallbackButton(
                  icon: Icon(
                    icon ?? Symbols.delete,
                    color: Colors.warningPrimaryColor,
                    size: (size * 0.8).clamp(0, 48),
                  ),
                  onPressed: isLoading ? null : onConfirm,
                ),
                SizedBox(height: size * 0.2, width: size * 0.2),
                BandScreenFallbackButton(
                  icon: Icon(
                    icon ?? Symbols.close,
                    size: (size * 0.2).clamp(0, 32),
                  ),
                  onPressed: isLoading ? null : () => $close(null),
                ),
              ];

              return Flex(
                mainAxisSize: MainAxisSize.max,
                mainAxisAlignment: MainAxisAlignment.center,
                crossAxisAlignment: CrossAxisAlignment.center,
                direction: activeBreakpoint == DeviceType.dock
                    ? Axis.vertical
                    : Axis.horizontal,
                children: children,
              );
            },
          );
        }

        return child;
      },
    );
    return PopScope(canPop: !isLoading, child: dialog);
  }
}
