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
    return SmallerOrEqualTo(
      breakpoint: DeviceType.band,
      builder: (context, isBand) {
        if (isBand) {
          return Column(
            mainAxisSize: MainAxisSize.max,
            mainAxisAlignment: MainAxisAlignment.center,
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              IconButton(
                icon: LayoutBuilder(
                  builder: (context, constraint) {
                    return Icon(
                      icon ?? Symbols.delete,
                      color: Colors.warningPrimaryColor,
                      size: (constraint.maxWidth * 0.8).clamp(0, 48),
                    );
                  },
                ),
                onPressed: onConfirm,
              ),
              LayoutBuilder(builder: (context, constraint) {
                return SizedBox(height: constraint.maxWidth * 0.2);
              }),
              IconButton(
                icon: LayoutBuilder(
                  builder: (context, constraint) {
                    return Icon(
                      icon ?? Symbols.close,
                      size: (constraint.maxWidth * 0.2).clamp(0, 32),
                    );
                  },
                ),
                onPressed: () => Navigator.pop(context, null),
              ),
            ],
          );
        }

        return child;
      },
    );
  }
}
