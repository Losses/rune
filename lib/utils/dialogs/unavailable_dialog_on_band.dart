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
        // Keep one stable subtree as the viewport changes. On very small windows the
        // dialog remains reachable in both directions rather than unmounting its form.
        return LayoutBuilder(
          builder: (context, constraints) {
            final width = max(320.0, constraints.maxWidth);
            final height = max(
              dialogMinimumViewportHeight,
              constraints.maxHeight,
            );
            return SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: SizedBox(
                width: width,
                child: SingleChildScrollView(
                  child: SizedBox(
                    width: width,
                    height: height,
                    child: widget.child,
                  ),
                ),
              ),
            );
          },
        );
      },
    );
  }
}
