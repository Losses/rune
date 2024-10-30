import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../providers/responsive_providers.dart';

class RemoveDialogOnBand extends StatelessWidget {
  const RemoveDialogOnBand({
    super.key,
    required this.child,
    required this.onConfirm,
    this.icon,
  });

  final IconData? icon;
  final Widget child;
  final VoidCallback onConfirm;

  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.belt,
        DeviceType.tv
      ],
      builder: (context, activeBreakpoint) {
        if (activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.belt ||
            activeBreakpoint == DeviceType.dock) {
          return LayoutBuilder(
            builder: (context, constraint) {
              final size = min(constraint.maxWidth, constraint.maxHeight);

              final children = [
                IconButton(
                  icon: Icon(
                    icon ?? Symbols.delete,
                    color: Colors.warningPrimaryColor,
                    size: (size * 0.8).clamp(0, 48),
                  ),
                  onPressed: onConfirm,
                ),
                SizedBox(height: size * 0.2, width: size * 0.2),
                IconButton(
                  icon: Icon(
                    icon ?? Symbols.close,
                    size: (size * 0.2).clamp(0, 32),
                  ),
                  onPressed: () => Navigator.pop(context, null),
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
  }
}
