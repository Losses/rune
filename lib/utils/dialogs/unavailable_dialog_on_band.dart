import 'dart:math';

import 'package:fluent_ui/fluent_ui.dart';
import 'package:material_symbols_icons/symbols.dart';

import '../../widgets/band_screen_fallback_button.dart';
import '../../providers/responsive_providers.dart';

/// Minimum virtual height used when an open dialog is larger than the window.
const double dialogMinimumViewportHeight = 480;

class UnavailableDialogOnBand extends StatefulWidget {
  const UnavailableDialogOnBand({
    super.key,
    required this.child,
    this.icon,
    required this.$close,
  });

  final IconData? icon;
  final Widget child;
  final void Function(Null) $close;

  @override
  State<UnavailableDialogOnBand> createState() =>
      _UnavailableDialogOnBandState();
}

class _UnavailableDialogOnBandState extends State<UnavailableDialogOnBand> {
  bool _hasShownDialog = false;

  @override
  Widget build(BuildContext context) {
    return DeviceTypeBuilder(
      deviceType: const [
        DeviceType.band,
        DeviceType.dock,
        DeviceType.belt,
        DeviceType.tv,
      ],
      builder: (context, activeBreakpoint) {
        final compact =
            activeBreakpoint == DeviceType.band ||
            activeBreakpoint == DeviceType.belt ||
            activeBreakpoint == DeviceType.dock;
        if (compact && !_hasShownDialog) {
          return Center(
            child: LayoutBuilder(
              builder: (context, constraint) {
                return BandScreenFallbackButton(
                  icon: Icon(
                    widget.icon ?? Symbols.devices,
                    size: (min(constraint.maxWidth, constraint.maxHeight) * 0.8)
                        .clamp(0, 48),
                  ),
                  onPressed: () => widget.$close(null),
                );
              },
            ),
          );
        }
        _hasShownDialog = true;
        return ScrollableDialogFrame(child: widget.child);
      },
    );
  }
}

/// Keeps a dialog that is already open usable when the window shrinks: one
/// stable subtree, laid out at least 320 px wide and
/// [dialogMinimumViewportHeight] high, that scrolls in both directions rather
/// than unmounting its form or overflowing.
class ScrollableDialogFrame extends StatelessWidget {
  const ScrollableDialogFrame({
    super.key,
    required this.child,
    this.minimumHeight = dialogMinimumViewportHeight,
  });

  final Widget child;

  /// The least height the dialog is laid out at; a dialog that fills the window
  /// by design passes 0 and is only kept from getting narrower than 320.
  final double minimumHeight;

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final width = max(320.0, constraints.maxWidth);
        final height = max(minimumHeight, constraints.maxHeight);
        return SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: SizedBox(
            width: width,
            child: SingleChildScrollView(
              child: SizedBox(width: width, height: height, child: child),
            ),
          ),
        );
      },
    );
  }
}
